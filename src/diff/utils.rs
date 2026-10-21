//! Static helper methods used by the diff pipeline: diff computation, word
//! boundary detection, V-C-V split point, and re-rendering.

use crate::buffers::{CharVec, OutBuffer};
use crate::diff::Diffable;
use crate::engine::UltraFastViEngine;
use crate::modes::{IS_TONE_KEY, InputMethod, Mode};

impl UltraFastViEngine {
    /// Compute minimal diff from `prev` → `new`, writing suffix into `out`.
    ///
    /// Returns `(backspaces, suffix_len)`. The caller sends `backspaces`
    /// delete keys then types the `suffix` chars to transform `prev` → `new`.
    pub(crate) fn diff_into(prev: &str, new: &str, out: &mut OutBuffer) -> (usize, usize) {
        // Single-pass: count common prefix and track prev char count
        // simultaneously, avoiding the double `.chars().count()` iteration.
        let mut common = 0usize;
        let mut prev_count = 0usize;
        let mut prev_iter = prev.chars();
        let mut new_iter = new.chars();
        loop {
            let p = prev_iter.next();
            if p.is_some() {
                prev_count += 1;
            }
            match (p, new_iter.next()) {
                (Some(a), Some(b)) if a == b => common += 1,
                _ => break,
            }
        }
        // Count remaining prev chars (after the common prefix)
        for _ in prev_iter {
            prev_count += 1;
        }
        let backspaces = prev_count - common;
        out.clear();
        for c in new.chars().skip(common) {
            let _ = out.push(c);
        }
        (backspaces, out.len())
    }

    /// Returns true for characters that end the current composing word.
    ///
    /// Any ASCII non-alphanumeric character is a word boundary — this covers
    /// `/`, `\`, `-`, `_`, `@`, `#`, etc. that users type mid-sentence
    /// (e.g. URLs, paths, code). Without this, a leading `/` would be pushed
    /// into the buffer as a literal consonant, corrupting `is_legal_onset`
    /// and silently disabling tone/modifier application for the rest of the
    /// word (see bug #11: `/duowcs` produced `/duowcs` instead of `/được`).
    ///
    /// Digits are NOT boundaries because VNI uses `0-9` as tone/modifier keys.
    /// Non-ASCII characters (incl. precomposed Vietnamese) are NOT boundaries
    /// here — they are decomposed by `feed()` and flow through the normal path.
    /// Unicode whitespace is still a boundary via `is_whitespace()`.
    #[inline]
    pub(crate) fn is_word_boundary(ch: char) -> bool {
        ch.is_whitespace() || (ch.is_ascii() && !ch.is_ascii_alphanumeric())
    }

    /// Returns true if the composed output equals the raw input (no Vietnamese transforms).
    #[inline]
    pub(crate) fn is_raw_passthrough_slice(raw: &[char], composed: &str) -> bool {
        if raw.is_empty() {
            return true;
        }
        let mut ci = composed.chars();
        for &r in raw {
            match ci.next() {
                Some(c) if c == r => {}
                _ => return false,
            }
        }
        ci.next().is_none()
    }

    /// Find the V-C-V split point: index in raw_chars where the second syllable starts.
    pub(crate) fn find_split_point(raw: &[char]) -> usize {
        let n = raw.len();
        if n == 0 {
            return 0;
        }
        let new_vowel_pos = n - 1;
        let mut last_old_vowel = 0usize;
        let mut found_old_vowel = false;
        for i in (0..new_vowel_pos).rev() {
            if Self::is_ascii_vowel(raw[i] as u8) {
                last_old_vowel = i;
                found_old_vowel = true;
                break;
            }
        }
        if !found_old_vowel {
            return 0;
        }
        if last_old_vowel < new_vowel_pos {
            let run_start = (last_old_vowel + 1..new_vowel_pos)
                .find(|&i| !Self::is_ascii_vowel(raw[i] as u8))
                .unwrap_or(new_vowel_pos);
            // The consonant run may contain keys that were consumed as
            // tone/modifier marks on the first syllable ("neefbo": 'f' is
            // huyền on "nè", not an onset). The syllable boundary is the
            // longest suffix of the run that is a legal Vietnamese onset —
            // digraphs like "tr"/"ng" stay together, a stray tone key is
            // left in syllable 1.
            let run = &raw[run_start..new_vowel_pos];
            if run.len() > 1 {
                for take in (1..=run.len()).rev() {
                    let tail = &run[run.len() - take..];
                    let mut onset_raw = [0u8; 4];
                    let mut k = 0usize;
                    for &c in tail.iter().take(4) {
                        onset_raw[k] = c as u8;
                        k += 1;
                    }
                    if crate::tables::is_legal_onset(&onset_raw[..k]) {
                        return new_vowel_pos - take;
                    }
                }
            }
            return run_start;
        }
        0
    }

    /// Re-render a slice of chars through the engine and return rendered output.
    ///
    /// On `std`: uses a thread-local scratch engine to avoid allocating a
    /// fresh `UltraFastViEngine` on every V-C-V split.
    /// On `no_std`: creates a new engine each time (no `thread_local!`).
    ///
    /// Sets both `mode` AND `input_method` on the scratch engine so that
    /// `decompose_vietnamese_char` (which uses `input_method`) works correctly
    /// for VNI precomposed input.
    pub(crate) fn rerender_chars(
        raw: &[char],
        mode: &'static Mode,
        is_simple_telex: bool,
    ) -> OutBuffer {
        #[cfg(feature = "std")]
        {
            thread_local! {
                static SCRATCH: std::cell::RefCell<UltraFastViEngine> =
                    std::cell::RefCell::new(UltraFastViEngine::new());
            }
            SCRATCH.with(|s| {
                let mut eng = s.borrow_mut();
                eng.clear();
                eng.mode = mode;
                // Sync input_method with the mode so decompose_vietnamese_char
                // uses the correct Telex/VNI key mappings.
                eng.input_method = match mode.resolver {
                    crate::modes::ResolverKind::Telex => InputMethod::Telex,
                    crate::modes::ResolverKind::Vni => InputMethod::Vni,
                };
                eng.is_simple_telex = is_simple_telex;
                for &c in raw {
                    eng.feed(c);
                }
                eng.out_buf.clone()
            })
        }
        #[cfg(not(feature = "std"))]
        {
            let mut eng = UltraFastViEngine::new();
            eng.mode = mode;
            eng.input_method = match mode.resolver {
                crate::modes::ResolverKind::Telex => InputMethod::Telex,
                crate::modes::ResolverKind::Vni => InputMethod::Vni,
            };
            eng.is_simple_telex = is_simple_telex;
            for &c in raw {
                eng.feed(c);
            }
            eng.out_buf.clone()
        }
    }

    #[inline]
    pub(crate) fn is_ascii_vowel(b: u8) -> bool {
        matches!(b, b'a' | b'e' | b'i' | b'o' | b'u' | b'y')
    }

    #[inline]
    pub(crate) fn is_tone_key_in_mode(ch: char, mode: &Mode) -> bool {
        let b = ch as u8;
        mode.classify[b as usize] & IS_TONE_KEY != 0
    }

    /// True when the word's last two keystrokes are the same tone key
    /// ("mas**s**", "tim**ff**") — the standard Telex/VNI double-key cancel.
    /// The pipeline consumes the second key, so the raw word and the
    /// pipeline's raw diverge and the commit must not be treated as final.
    #[inline]
    pub(crate) fn ends_with_doubled_tone_key(word: &[char], mode: &Mode) -> bool {
        let n = word.len();
        n >= 2 && word[n - 1] == word[n - 2] && Self::is_tone_key_in_mode(word[n - 1], mode)
    }

    /// True when the keystrokes typed after a dictionary word read as
    /// Vietnamese: rendered on their own, the tail carries at least one
    /// Vietnamese transform (a tone or vowel-quality mark). English
    /// continuations stay literal — "key", "code", "less", "through" — while
    /// Vietnamese ones do not ("ter" → "tẻ", "tes" → "tés").
    #[inline]
    pub(crate) fn tail_reads_as_vietnamese(
        word_raw: &[char],
        word_len: usize,
        mode: &'static Mode,
        is_simple_telex: bool,
    ) -> bool {
        let tail: CharVec<24> = word_raw.iter().skip(word_len).copied().collect();
        if tail.is_empty() {
            return false;
        }
        !Self::rerender_chars(&tail, mode, is_simple_telex).is_ascii()
    }

    /// True when the word re-rendered as pure Vietnamese (no dictionary
    /// override) stays plain letters — no Vietnamese diacritics. An English
    /// word that would come out as "rétlesnes" is never what the user meant,
    /// so the sticky raw passthrough keeps it.
    ///
    /// Runs a scratch replay of the word with the override disabled: only
    /// reached while a cancel fallback is armed, never on the normal path.
    pub(crate) fn word_renders_plain(&self, word_raw: &[char]) -> bool {
        let mut scratch = UltraFastViEngine::new();
        scratch.set_input_method(self.input_method());
        scratch.set_modern_orthography(self.modern_orthography());
        scratch.set_quick_start(self.quick_start());
        scratch.set_quick_telex(self.quick_telex());
        scratch.set_relaxed_coda(self.relaxed_coda());
        scratch.set_english_override(false);
        for &c in word_raw {
            let _ = scratch.feed_diff(c);
        }
        scratch.current_output_diff().is_ascii()
    }

    /// Pop the last raw keystroke of the current word.
    ///
    /// Drops the saved English fallback once the word shrinks below the
    /// dictionary word it was armed for: the saved pipeline snapshot no
    /// longer matches the typed prefix, so restoring it would rewrite text
    /// the user has already deleted.
    #[inline]
    pub(crate) fn pop_word_raw(&mut self) -> Option<char> {
        let popped = self.diff.word_raw.pop();
        if let Some(fb) = self.diff.english_fallback.as_ref() {
            if self.diff.word_raw.len() < fb.word_len {
                self.diff.english_fallback = None;
            }
        }
        popped
    }

    #[inline]
    pub(crate) fn is_single_consonant_appended_slice(
        raw: &[char],
        last_valid_raw_len: usize,
    ) -> bool {
        if raw.len() != last_valid_raw_len + 1 {
            return false;
        }
        let ch = raw[last_valid_raw_len];
        !Self::is_ascii_vowel(ch as u8)
    }

    /// Find the raw index where the coda starts (index past the last vowel).
    /// If there is no vowel, the whole slice is treated as onset/coda.
    #[inline]
    pub(crate) fn raw_coda_start(raw: &[char]) -> usize {
        let mut last_vowel = None;
        for (i, &c) in raw.iter().enumerate() {
            if Self::is_ascii_vowel(c as u8) {
                last_vowel = Some(i);
            }
        }
        last_vowel.map(|i| i + 1).unwrap_or(0)
    }
}
