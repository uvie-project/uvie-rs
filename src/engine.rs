use crate::buffers::{OutBuffer, StackStr, new_out_buffer};
use crate::composing::Composable;
use crate::diff::DiffState;
use crate::modes::{InputMethod, Mode, mode_for};
use crate::syllable::{SylBuf, SylStructure};

// ---------------------------------------------------------------------------
// Public engine struct
// ---------------------------------------------------------------------------

pub struct UltraFastViEngine {
    /// Per-char buffer for the current composing word.
    pub(crate) buf: SylBuf,
    /// Raw keystroke snapshot. `raw[i]` == lowercased byte of the i-th key.
    pub(crate) raw: [u8; 24],
    /// Uppercase flag for each raw keystroke.
    pub(crate) raw_caps: [bool; 24],
    pub(crate) raw_len: usize,
    /// Rendered output of the current composing word.
    pub(crate) out_buf: OutBuffer,
    /// Accumulated committed text (prior complete words + boundary chars).
    pub(crate) committed: OutBuffer,
    /// Input method - determines classifier and tone tables.
    pub(crate) input_method: InputMethod,
    pub(crate) mode: &'static Mode,
    /// Engine configuration flags.
    pub(crate) enable_quick_start: bool,
    pub(crate) enable_quick_telex: bool,
    pub(crate) enable_modern_orthography: bool,
    pub(crate) enable_relaxed_coda: bool,
    /// English dictionary override: when a typed word matches the English
    /// override table, show the raw English word instead of the garbled
    /// Vietnamese transform. Default on; hosts can expose it as a
    /// "restore English words" user toggle.
    pub(crate) enable_english_override: bool,
    /// Simple Telex mode: `w` always applies horn/breve to all vowel
    /// candidates (vneHookAll), with no standalone `w→ư`, no double-w
    /// cancel, and no silent consume. Matches UniKey "Simple Telex".
    pub(crate) is_simple_telex: bool,

    /// Incrementally maintained syllable structure (onset/nucleus/coda slots).
    pub(crate) syl_structure: SylStructure,

    /// Diff-mode state (V-C-V splitting, screen diffing).
    pub(crate) diff: DiffState,

    /// Cached `partition_syllable()` result + the `buf.version()` it was
    /// computed from. On each access, if the version matches, the cached
    /// result is returned; otherwise it's recomputed. This avoids 5-10
    /// redundant O(n) scans per keystroke.
    pub(crate) cached_partition: (u32, (usize, usize, usize, usize)),
}

impl UltraFastViEngine {
    pub fn new() -> Self {
        let input_method = InputMethod::Telex;
        Self {
            buf: SylBuf::new(),
            raw: [0u8; 24],
            raw_caps: [false; 24],
            raw_len: 0,
            out_buf: new_out_buffer(),
            committed: new_out_buffer(),
            input_method,
            mode: mode_for(input_method),
            enable_quick_start: false,
            enable_quick_telex: false,
            enable_modern_orthography: false,
            enable_relaxed_coda: false,
            enable_english_override: true,
            is_simple_telex: false,
            syl_structure: SylStructure::new(),
            diff: DiffState::new(),
            cached_partition: (0, (0, 0, 0, 0)),
        }
    }

    // ------------------------------------------------------------------
    // Configuration accessors
    // ------------------------------------------------------------------

    pub fn set_quick_start(&mut self, enabled: bool) {
        self.enable_quick_start = enabled;
    }
    pub fn quick_start(&self) -> bool {
        self.enable_quick_start
    }

    pub fn set_quick_telex(&mut self, enabled: bool) {
        self.enable_quick_telex = enabled;
    }
    pub fn quick_telex(&self) -> bool {
        self.enable_quick_telex
    }

    pub fn set_modern_orthography(&mut self, enabled: bool) {
        self.enable_modern_orthography = enabled;
    }
    pub fn modern_orthography(&self) -> bool {
        self.enable_modern_orthography
    }

    pub fn set_relaxed_coda(&mut self, enabled: bool) {
        self.enable_relaxed_coda = enabled;
    }
    pub fn relaxed_coda(&self) -> bool {
        self.enable_relaxed_coda
    }

    pub fn set_english_override(&mut self, enabled: bool) {
        self.enable_english_override = enabled;
    }
    pub fn english_override(&self) -> bool {
        self.enable_english_override
    }

    pub fn set_input_method(&mut self, method: InputMethod) {
        self.input_method = method;
        self.mode = mode_for(method);
        self.is_simple_telex = method == InputMethod::SimpleTelex;
    }
    pub fn input_method(&self) -> InputMethod {
        self.input_method
    }

    // ------------------------------------------------------------------
    // State queries
    // ------------------------------------------------------------------

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty() && self.committed.is_empty()
    }

    pub fn is_composing(&self) -> bool {
        !self.buf.is_empty()
    }

    /// Current logical raw length (may differ from push count due to double-cancel).
    pub fn raw_len(&self) -> usize {
        self.raw_len
    }

    /// Length of the diff-mode raw character buffer.
    pub fn raw_chars_len(&self) -> usize {
        self.diff.raw_chars.len()
    }

    /// Copy the diff-mode raw characters into a `StackStr` for debugging/tests.
    /// Returns a stack-allocated buffer (no heap allocation).
    /// Uses 128 bytes (not 64) because 24 raw chars can be up to 96 bytes
    /// for non-ASCII/precomposed Vietnamese characters.
    pub fn raw_chars_string(&self) -> StackStr<128> {
        let mut buf = StackStr::new();
        for &c in self.diff.raw_chars.iter() {
            let _ = buf.push(c);
        }
        buf
    }

    /// Copy the lossless raw keystrokes of the ENTIRE current word into a
    /// `StackStr` — across V-C-V syllable splits, double-tone cancels and
    /// English dictionary commits. Unlike `raw_chars_string` (the current
    /// composing syllable only), this is what a host "escape restores the
    /// literal keystrokes" action needs: "master" renders "mátẻ", but the
    /// user typed "master".
    ///
    /// Returns an EMPTY buffer when the word outgrew the 24-keystroke raw
    /// buffer: its prefix was committed mid-word, so restoring would delete
    /// text the raw no longer covers. Hosts must treat empty as "cannot
    /// restore" and leave the render alone.
    pub fn word_raw_string(&self) -> StackStr<128> {
        let mut buf = StackStr::new();
        if self.diff.word_raw_overflowed {
            return buf;
        }
        for &c in self.diff.word_raw.iter() {
            let _ = buf.push(c);
        }
        buf
    }

    pub fn current_composing(&self) -> &str {
        &self.out_buf
    }

    /// Returns the classify flags for a raw byte in the current input mode.
    #[inline]
    pub fn mode_classify(&self, b: u8) -> u8 {
        self.mode.classify[b as usize]
    }

    pub fn committed_text(&self) -> &str {
        &self.committed
    }

    /// Returns the full output (committed + composing) as a stack-allocated
    /// buffer. No heap allocation.
    pub fn current_output(&self) -> StackStr<256> {
        let mut buf = StackStr::new();
        let _ = buf.push_str(&self.committed);
        let _ = buf.push_str(&self.out_buf);
        buf
    }

    /// Returns the full diff-mode output (diff_committed + prev_rendered) as a
    /// stack-allocated buffer. This is what's actually on screen in diff mode.
    /// `current_output()` returns the legacy `committed + out_buf` which is
    /// usually empty in diff mode and does NOT reflect the visible text.
    pub fn current_output_diff(&self) -> StackStr<256> {
        let mut buf = StackStr::new();
        let _ = buf.push_str(&self.diff.diff_committed);
        let _ = buf.push_str(&self.diff.prev_rendered);
        buf
    }

    /// Returns the current syllable structure (onset/nucleus/coda slots).
    #[inline]
    pub fn syl_structure(&self) -> &SylStructure {
        &self.syl_structure
    }

    // ------------------------------------------------------------------
    // Lifecycle
    // ------------------------------------------------------------------

    pub fn clear(&mut self) {
        self.buf.clear();
        self.raw_len = 0;
        self.out_buf.clear();
        self.committed.clear();
        self.syl_structure.clear();
        // No need to reset cached_partition — version check handles it.
    }

    /// Finalise the composing word into `committed` and reset composing state.
    pub fn commit(&mut self) {
        self.render_out_buf();
        let _ = self.committed.push_str(&self.out_buf);
        self.buf.clear();
        self.raw_len = 0;
        self.out_buf.clear();
    }

    /// Delete the last typed key (backspace). Returns the new composing text.
    pub fn backspace(&mut self) -> &str {
        if self.raw_len > 0 {
            self.raw_len -= 1;
            let target_len = self.raw_len;
            self.buf.clear();
            self.raw_len = 0;
            for i in 0..target_len {
                let b = self.raw[i];
                let caps = self.raw_caps[i];
                self.raw[self.raw_len] = b;
                self.raw_caps[self.raw_len] = caps;
                self.raw_len += 1;
                self.process_key(b, caps);
            }
            self.render_out_buf();
            return &self.out_buf;
        }
        if !self.committed.is_empty() {
            self.committed.pop();
        }
        self.out_buf.clear();
        &self.out_buf
    }

    // ------------------------------------------------------------------
    // Core feed method
    // ------------------------------------------------------------------

    /// Feed one character. Returns the current composing text (not including
    /// committed text). Whitespace commits the current word.
    pub fn feed(&mut self, key: char) -> &str {
        if key.is_whitespace() {
            self.render_out_buf();
            let _ = self.committed.push_str(&self.out_buf);
            self.buf.clear();
            self.raw_len = 0;
            self.out_buf.clear();
            let _ = self.committed.push(key);
            return &self.out_buf;
        }

        // Pre-composed Vietnamese characters (e.g. "ý", "ê", "đ") are decomposed
        // into their ASCII keystroke equivalents so the engine can process copy-
        // pasted or composed text the same way as live keystrokes.
        if let Some((base, modifier, tone)) = decompose_vietnamese_char(key, self.input_method) {
            let base_caps = base != base.to_ascii_lowercase();
            self.push_raw_key(base.to_ascii_lowercase() as u8, base_caps);
            if let Some(m) = modifier {
                self.push_raw_key(m, false);
            }
            if let Some(t) = tone {
                self.push_raw_key(t, false);
            }
            self.render_out_buf();
            return &self.out_buf;
        }

        let lower = key.to_ascii_lowercase();
        let caps = key != lower;

        if self.enable_quick_start {
            match lower {
                'j' => {
                    self.push_raw_key(b'g', false);
                    self.push_raw_key(b'i', false);
                }
                'f' => {
                    self.push_raw_key(b'p', false);
                    self.push_raw_key(b'h', false);
                }
                'w' => {
                    self.push_raw_key(b'q', false);
                    self.push_raw_key(b'u', false);
                }
                _ => {
                    self.push_raw_key(lower as u8, caps);
                }
            }
        } else if self.enable_quick_telex && self.raw_len > 0 {
            let prev = self.raw[self.raw_len - 1];
            let expansion: Option<[u8; 2]> = match lower {
                'c' => Some([b'c', b'h']),
                'g' => Some([b'g', b'i']),
                'h' => Some([b'n', b'h']),
                'k' => Some([b'k', b'h']),
                'n' => Some([b'n', b'g']),
                'q' => Some([b'q', b'u']),
                'p' => Some([b'p', b'h']),
                't' => Some([b't', b'h']),
                _ => None,
            };
            if let Some(pair) = expansion {
                if prev == lower as u8 {
                    self.raw_len -= 1;
                    self.buf.pop();
                    self.push_raw_key(pair[0], false);
                    self.push_raw_key(pair[1], false);
                } else {
                    self.push_raw_key(lower as u8, caps);
                }
            } else {
                self.push_raw_key(lower as u8, caps);
            }
        } else {
            self.push_raw_key(lower as u8, caps);
        }

        self.render_out_buf();
        &self.out_buf
    }
}

/// Decompose a pre-composed Vietnamese character into its ASCII keystroke
/// components: base vowel/consonant, optional modifier key, and optional tone
/// key. Returns `None` for non-Vietnamese characters so the caller can fall
/// back to normal processing.
fn decompose_vietnamese_char(
    c: char,
    method: InputMethod,
) -> Option<(char, Option<u8>, Option<u8>)> {
    let is_upper = c.is_uppercase();
    // Keystroke codes for the two supported input methods.
    let (a_circumflex, e_circumflex, o_circumflex, breve, horn, dd, s, f, r, x, j) = match method {
        InputMethod::Telex | InputMethod::SimpleTelex => (
            b'a', b'e', b'o', b'w', b'w', b'd', b's', b'f', b'r', b'x', b'j',
        ),
        InputMethod::Vni => (
            b'6', b'6', b'6', b'8', b'7', b'9', b'1', b'2', b'3', b'4', b'5',
        ),
    };
    let (base, modifier, tone) = match c {
        // a
        'a' | 'A' => ('a', None, None),
        'á' | 'Á' => ('a', None, Some(s)),
        'à' | 'À' => ('a', None, Some(f)),
        'ả' | 'Ả' => ('a', None, Some(r)),
        'ã' | 'Ã' => ('a', None, Some(x)),
        'ạ' | 'Ạ' => ('a', None, Some(j)),
        'â' | 'Â' => ('a', Some(a_circumflex), None),
        'ấ' | 'Ấ' => ('a', Some(a_circumflex), Some(s)),
        'ầ' | 'Ầ' => ('a', Some(a_circumflex), Some(f)),
        'ẩ' | 'Ẩ' => ('a', Some(a_circumflex), Some(r)),
        'ẫ' | 'Ẫ' => ('a', Some(a_circumflex), Some(x)),
        'ậ' | 'Ậ' => ('a', Some(a_circumflex), Some(j)),
        'ă' | 'Ă' => ('a', Some(breve), None),
        'ắ' | 'Ắ' => ('a', Some(breve), Some(s)),
        'ằ' | 'Ằ' => ('a', Some(breve), Some(f)),
        'ẳ' | 'Ẳ' => ('a', Some(breve), Some(r)),
        'ẵ' | 'Ẵ' => ('a', Some(breve), Some(x)),
        'ặ' | 'Ặ' => ('a', Some(breve), Some(j)),
        // e
        'e' | 'E' => ('e', None, None),
        'é' | 'É' => ('e', None, Some(s)),
        'è' | 'È' => ('e', None, Some(f)),
        'ẻ' | 'Ẻ' => ('e', None, Some(r)),
        'ẽ' | 'Ẽ' => ('e', None, Some(x)),
        'ẹ' | 'Ẹ' => ('e', None, Some(j)),
        'ê' | 'Ê' => ('e', Some(e_circumflex), None),
        'ế' | 'Ế' => ('e', Some(e_circumflex), Some(s)),
        'ề' | 'Ề' => ('e', Some(e_circumflex), Some(f)),
        'ể' | 'Ể' => ('e', Some(e_circumflex), Some(r)),
        'ễ' | 'Ễ' => ('e', Some(e_circumflex), Some(x)),
        'ệ' | 'Ệ' => ('e', Some(e_circumflex), Some(j)),
        // i
        'i' | 'I' => ('i', None, None),
        'í' | 'Í' => ('i', None, Some(s)),
        'ì' | 'Ì' => ('i', None, Some(f)),
        'ỉ' | 'Ỉ' => ('i', None, Some(r)),
        'ĩ' | 'Ĩ' => ('i', None, Some(x)),
        'ị' | 'Ị' => ('i', None, Some(j)),
        // o
        'o' | 'O' => ('o', None, None),
        'ó' | 'Ó' => ('o', None, Some(s)),
        'ò' | 'Ò' => ('o', None, Some(f)),
        'ỏ' | 'Ỏ' => ('o', None, Some(r)),
        'õ' | 'Õ' => ('o', None, Some(x)),
        'ọ' | 'Ọ' => ('o', None, Some(j)),
        'ô' | 'Ô' => ('o', Some(o_circumflex), None),
        'ố' | 'Ố' => ('o', Some(o_circumflex), Some(s)),
        'ồ' | 'Ồ' => ('o', Some(o_circumflex), Some(f)),
        'ổ' | 'Ổ' => ('o', Some(o_circumflex), Some(r)),
        'ỗ' | 'Ỗ' => ('o', Some(o_circumflex), Some(x)),
        'ộ' | 'Ộ' => ('o', Some(o_circumflex), Some(j)),
        'ơ' | 'Ơ' => ('o', Some(horn), None),
        'ớ' | 'Ớ' => ('o', Some(horn), Some(s)),
        'ờ' | 'Ờ' => ('o', Some(horn), Some(f)),
        'ở' | 'Ở' => ('o', Some(horn), Some(r)),
        'ỡ' | 'Ỡ' => ('o', Some(horn), Some(x)),
        'ợ' | 'Ợ' => ('o', Some(horn), Some(j)),
        // u
        'u' | 'U' => ('u', None, None),
        'ú' | 'Ú' => ('u', None, Some(s)),
        'ù' | 'Ù' => ('u', None, Some(f)),
        'ủ' | 'Ủ' => ('u', None, Some(r)),
        'ũ' | 'Ũ' => ('u', None, Some(x)),
        'ụ' | 'Ụ' => ('u', None, Some(j)),
        'ư' | 'Ư' => ('u', Some(horn), None),
        'ứ' | 'Ứ' => ('u', Some(horn), Some(s)),
        'ừ' | 'Ừ' => ('u', Some(horn), Some(f)),
        'ử' | 'Ử' => ('u', Some(horn), Some(r)),
        'ữ' | 'Ữ' => ('u', Some(horn), Some(x)),
        'ự' | 'Ự' => ('u', Some(horn), Some(j)),
        // y
        'y' | 'Y' => ('y', None, None),
        'ý' | 'Ý' => ('y', None, Some(s)),
        'ỳ' | 'Ỳ' => ('y', None, Some(f)),
        'ỷ' | 'Ỷ' => ('y', None, Some(r)),
        'ỹ' | 'Ỹ' => ('y', None, Some(x)),
        'ỵ' | 'Ỵ' => ('y', None, Some(j)),
        // d with stroke
        'đ' | 'Đ' => ('d', Some(dd), None),
        _ => return None,
    };
    let base = if is_upper {
        base.to_ascii_uppercase()
    } else {
        base
    };
    Some((base, modifier, tone))
}

impl Default for UltraFastViEngine {
    fn default() -> Self {
        Self::new()
    }
}
