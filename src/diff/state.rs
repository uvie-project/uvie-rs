//! Diff-mode state: snapshot stack and tracking buffers.
//!
//! All fields are stack-allocated (no heap) — see `buffers.rs` for the
//! underlying `CharVec` / `OutBuffer` types.

use crate::buffers::{CharVec, OutBuffer, StackStr};
use crate::syllable::SylBuf;

/// Sentence-level language hint supplied by the host — e.g. Apple's
/// NaturalLanguage recognizer run over `sentence_text()`.
///
/// `Auto` keeps the engine's own rules. `Vietnamese` and `English` force the
/// sentence language at commit time; both reset to `Auto` at a sentence
/// boundary, because the recognizer's verdict describes the sentence that
/// just ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(u8)]
pub enum ContextHint {
    #[default]
    Auto = 0,
    Vietnamese = 1,
    English = 2,
}

impl ContextHint {
    /// Decode the FFI's `u8` (`uvie_engine_set_context_hint`).
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Vietnamese,
            2 => Self::English,
            _ => Self::Auto,
        }
    }
}

/// Snapshot of the composing state after a keystroke, used for O(1) backspace.
/// All fields are stack-allocated (no heap).
#[derive(Clone)]
pub struct ComposingSnapshot {
    pub buf: SylBuf,
    pub raw_len: usize,
    pub raw_chars: CharVec<24>,
    pub key_log: CharVec<24>,
    pub prev_rendered: OutBuffer,
    pub prev_inner_render: OutBuffer,
    pub last_valid_raw_len: usize,
    pub last_valid_coda_start: usize,
    pub last_valid_out: OutBuffer,
    /// A double-press tone-key cancel already fired in the composing word
    /// (see `DiffState::word_had_cancel`).
    pub word_had_cancel: bool,
    /// A modifier-key cancel already fired in the composing word (see
    /// `DiffState::word_had_modifier_cancel`).
    pub word_had_modifier_cancel: bool,
}

/// A committed word on screen whose render carries a Vietnamese transform
/// (a mark or đ), or a top Vietnamese particle of two letters or more that is
/// not an English function word ("em", "anh", "nha", "cho"): the sentence is
/// Vietnamese.
///
/// A marked render whose keystrokes spell a known English word is NOT
/// evidence: "thí" is "this" waiting for the retroactive English fix, and
/// treating it as Vietnamese would freeze the whole English sentence.
fn is_vietnamese_evidence(raw: &[char], rendered: &str) -> bool {
    // A raw/restored word is ASCII; any non-ASCII letter is a mark or đ the
    // pipeline applied, which no English spelling produces.
    if !rendered.is_ascii() {
        if !rendered.chars().any(|c| c.is_alphabetic()) {
            return false;
        }
        // The render must be a real Vietnamese word, not the marks the
        // pipeline left on an English one ("mátẻ" is "master"), and its
        // keystrokes must not spell an English word: "thí" is "this" waiting
        // for the retroactive English fix.
        return (crate::tables::is_top_vietnamese_syllable(rendered)
            || crate::tables::is_common_vietnamese_syllable(rendered))
            && !(crate::tables::is_english_override(raw)
                || crate::tables::is_english_ambiguous_word(raw)
                || crate::tables::is_english_doubled_key_word(raw));
    }
    // A top Vietnamese particle of two letters or more that is not an English
    // function word ("em", "anh", "nha", "cho"): a sentence that already has
    // one is Vietnamese.
    rendered.chars().count() >= 2
        && crate::tables::is_top_vietnamese_syllable(rendered)
        && !crate::tables::is_english_function_word(rendered)
}

/// A separator char that ends a sentence. A `.` is only one when it is not a
/// code/URL dot or glued between two letters ("a.b") — the caller checks.
fn is_sentence_end(ch: char) -> bool {
    matches!(ch, '.' | '!' | '?' | '\n' | '\r')
}

/// A committed word kept for post-commit editing (arrow-back onto the word
/// and typing a modifier/tone key re-renders it in place — LabanKey-style).
/// All fields are stack-allocated (no heap beyond the OutBuffer's inline
/// storage on no_std / String on std).
#[derive(Clone)]
pub struct CommittedWord {
    /// Raw keystrokes of the committed word (lossless, case-preserved).
    pub raw: CharVec<24>,
    /// Rendered text that is on screen for this word.
    pub rendered: OutBuffer,
    /// Separator chars (space / punctuation / Enter) on screen right after
    /// this word — the ones between it and the next newer word, or, for the
    /// newest entry, the trailing ones typed after it. The host reports them
    /// through `note_separator`/`remove_separator`; together with `rendered`
    /// they are the whole caret geometry (`caret_back` from the newest end).
    pub gap: usize,
    /// The last separator char the host reported after this word. With
    /// `gap == 1` it is the exact character between this word and the next
    /// one, which the retroactive English fix needs to rebuild the span.
    pub sep: char,
    /// `raw` still types `rendered`. Cleared when a native Backspace left a
    /// text whose canonical keystrokes do not render back to it (English
    /// "wa": a leading `w` is the ư modifier). A non-editable entry keeps its
    /// geometry but declines post-commit edits; native deletes keep shrinking
    /// its rendered text.
    pub editable: bool,
    /// The word was typed with a double-press tone-key cancel ("carr"), so
    /// its `raw` holds a key the user cancelled. The retroactive English fix
    /// must never rewrite this entry back to that raw.
    pub had_cancel: bool,
    /// A sentence-ending separator (`.`, `!`, `?`, newline) was typed right
    /// after this word: the next word starts a new sentence.
    pub ends_sentence: bool,
}

/// Pre-override Vietnamese pipeline state, saved when the English
/// dictionary override commits a word whose keystrokes the pipeline
/// collapsed. A doubled Telex key is the common case: "mass" commits the
/// raw word, but the pipeline's own raw is "mas" — the second `s` cancelled
/// the sắc. Committing pins the rest of the word to raw, so without this
/// state the cancel sequence could never be typed again ("masster" stayed
/// literal instead of rendering "master").
///
/// Restored when a consonant tail can no longer become a dictionary word:
/// the pipeline resumes exactly where the override interrupted it and the
/// whole word is re-rendered as Vietnamese.
#[derive(Clone)]
pub struct EnglishFallback {
    /// Pipeline state right after the keystroke that fired the override.
    pub snap: ComposingSnapshot,
    /// V-C-V committed prefix that was on screen before the override.
    pub diff_committed: OutBuffer,
    /// `word_raw.len()` when the override fired — the tail starts here.
    pub word_len: usize,
}

/// Diff-mode state: tracks what's on screen vs what the engine produced.
pub struct DiffState {
    /// Raw keystroke buffer for feed_diff (chars, not bytes; needed for V-C-V split).
    /// This is the "live" buffer used by the forward path; it is mutated by
    /// double-tone-cancel (swap+truncate) and V-C-V split (replaced by the new
    /// syllable), so it is NOT a faithful record of what the user typed.
    pub raw_chars: CharVec<24>,
    /// Lossless keystroke log for the current *composing* portion only.
    /// Unlike `raw_chars`, this is NEVER truncated/swap'd on double-tone-cancel,
    /// so backspace can faithfully rebuild the composing state by replaying it.
    /// It is reset on word boundary, full-buffer, V-C-V split (set to the new
    /// syllable, mirroring `raw_chars`), commit and reset.
    pub key_log: CharVec<24>,
    /// Full raw keystrokes for the entire current word (never truncated by
    /// V-C-V split or double-tone-cancel). Used for English dictionary
    /// override check at word boundaries. Cleared on word boundary, commit,
    /// reset, and full-buffer overflow.
    pub word_raw: CharVec<24>,
    /// Composing text currently visible on screen (for diffing).
    pub prev_rendered: OutBuffer,
    /// The inner engine's true last render (diff baseline).
    pub prev_inner_render: OutBuffer,
    /// Raw char count at the last valid (non-passthrough) Vietnamese render.
    pub last_valid_raw_len: usize,
    /// Coda start index at the last valid Vietnamese render (used to avoid optimistic display when the syllable already has a coda).
    pub last_valid_coda_start: usize,
    /// Output at the last valid Vietnamese render, used for V-C-V split.
    pub last_valid_out: OutBuffer,
    /// Accumulated auto-committed text from V-C-V splits (diff mode only).
    pub diff_committed: OutBuffer,
    /// Scratch buffer backing the &str returned by feed_diff/backspace_diff.
    pub diff_suffix: OutBuffer,
    /// Scratch buffer for optimistic candidate (avoids heap alloc per keystroke).
    pub scratch_optimistic: OutBuffer,
    /// Scratch buffer for display_composed (avoids heap alloc per keystroke).
    pub scratch_display: OutBuffer,
    /// Snapshot stack: one entry per keystroke, enabling O(1) backspace
    /// instead of O(n) replay. Cleared on word boundary, commit, V-C-V split.
    pub snapshots: [Option<ComposingSnapshot>; 24],
    pub snapshot_count: usize,
    /// Set when the current word outgrew the 24-keystroke `word_raw` buffer
    /// (its prefix was committed mid-word). `word_raw` then describes only
    /// the tail, so hosts must not restore it as the word's literal input.
    pub word_raw_overflowed: bool,
    /// Sticky English passthrough: set when the per-keystroke dictionary
    /// override fires. While set, the rest of the current word passes
    /// through raw — the suffix must not be re-transformed as Vietnamese
    /// (e.g. "perm"+"ission" would render "permision": the double-s
    /// cancel eats a letter and tone keys corrupt the tail). The raw tail
    /// accumulates in `prev_rendered`, keeping the on-screen invariant
    /// (diff_committed + prev_rendered) and the boundary commit path intact.
    /// Cleared on word boundary, commit, reset, and full-word erase.
    pub english_sticky: bool,
    /// Pipeline state saved when the override committed a word whose raw
    /// keystrokes the pipeline collapsed (see `EnglishFallback`). While set,
    /// the sticky tail is checked against the dictionary prefixes and the
    /// word falls back to Vietnamese rendering once it can no longer become
    /// an English word.
    pub english_fallback: Option<EnglishFallback>,
    /// Committed-word ring (oldest → newest), for post-commit editing.
    /// `edit_at_caret_diff` walks all entries to resolve the caret offset and
    /// updates the target IN PLACE (newer entries stay editable). Cleared on
    /// commit-adjacent invalidation paths (reset, full clear).
    pub edit_history: [Option<CommittedWord>; 8],
    /// Index of the oldest entry in the ring.
    pub edit_history_start: usize,
    /// Number of live entries in the ring.
    pub edit_history_len: usize,
    /// Caret offset (`caret_back`, screen chars from the end of the newest
    /// entry) the host must adopt after the last successful edit. Negative
    /// means the caret is right of the new newest word's end (the edited word
    /// was erased, leaving the caret inside the merged separators).
    pub last_edit_caret_back: isize,
    /// The last separator char the host reported (`'\0'` when none yet). A
    /// code char (`_`, '`', '=', ...) glued to the text before it — no space
    /// in between — marks the next word as code, not Vietnamese.
    pub last_sep: char,
    /// That separator char was typed directly after the previous text (no
    /// space before it): `foo_bar`, `a.b`, `x=1`.
    pub last_sep_glued: bool,
    /// A word was committed since the last separator: the next separator char
    /// is glued to that word (`foo_`, `bar.`).
    pub word_since_sep: bool,
    /// The composing word started right after a glued code separator: it is a
    /// code identifier and shows its literal keystrokes.
    pub word_after_code: bool,
    /// A double-press tone-key cancel fired in the composing word ("carr":
    /// the second `r` cancelled the hỏi). The user asked for the cancelled
    /// render, so a commit must not restore the keystrokes that produced it.
    pub word_had_cancel: bool,
    /// A modifier-key double-press cancel fired in the composing word ("aww":
    /// the second `w` cancelled the breve, "aas" after an `aa` → `â`). Unlike
    /// a doubled tone key — which is genuine English spelling ("off",
    /// "diffed", "mass") — a doubled modifier never belongs to an English
    /// word, so every rebuild from the raw keystrokes drops the consumed key
    /// ("awware" → "aware", "newws" → "news").
    pub word_had_modifier_cancel: bool,
    /// The current sentence already contains a Vietnamese word: its render
    /// carries Vietnamese marks or đ, or it is a top Vietnamese particle
    /// ("em", "anh", "nha"). Ambiguous words in such a sentence keep their
    /// Vietnamese reading. Reset at a sentence boundary and on a full reset.
    pub sentence_vi: bool,
    /// Host-supplied sentence-language hint (NaturalLanguage). Reset to
    /// `Auto` together with `sentence_vi`.
    pub context_hint: ContextHint,
    /// A `.` ended the previous word: it is a sentence boundary unless the
    /// next word starts immediately after it ("a.b" — a code token, not
    /// prose). Resolved by the first keystroke of the next word.
    pub sentence_break_pending: bool,
    /// The current whitespace-delimited token is a URL / path / code token:
    /// it contains `://`, starts with `/`, `~/` or `./`, has a `.` before a
    /// `/` (`github.com/x`), or a code char glued to its text (`signed-by=`).
    /// Every word inside it shows its literal keystrokes.
    pub token_url: bool,
    /// Chars seen in the current token since the last whitespace (separator
    /// chars only: the words themselves are not reported).
    pub token_len: u8,
    /// A word was committed inside the current token (the token does not
    /// start with the next separator char: `tieengs/vieetj` is a word and a
    /// slash, not a path).
    pub token_has_word: bool,
    /// First char of the current token (`/`, `~`, `.`, `[`, ...).
    pub token_first: char,
    /// The last two chars of the current token, for the `://` sequence.
    pub token_prev: char,
    pub token_prev2: char,
    /// A `.` was seen in the current token (`github.com/x`).
    pub token_dot: bool,
    /// The composing word started inside a URL / path token: it is an
    /// identifier, not Vietnamese.
    pub word_in_url: bool,
}

impl Default for DiffState {
    fn default() -> Self {
        Self::new()
    }
}

impl DiffState {
    pub const fn new() -> Self {
        Self {
            raw_chars: CharVec::new(),
            key_log: CharVec::new(),
            word_raw: CharVec::new(),
            prev_rendered: OutBuffer::new(),
            prev_inner_render: OutBuffer::new(),
            last_valid_raw_len: 0,
            last_valid_coda_start: 0,
            last_valid_out: OutBuffer::new(),
            diff_committed: OutBuffer::new(),
            diff_suffix: OutBuffer::new(),
            scratch_optimistic: OutBuffer::new(),
            scratch_display: OutBuffer::new(),
            snapshots: [const { None }; 24],
            snapshot_count: 0,
            word_raw_overflowed: false,
            english_sticky: false,
            english_fallback: None,
            edit_history: [const { None }; 8],
            edit_history_start: 0,
            edit_history_len: 0,
            last_edit_caret_back: 0,
            last_sep: '\0',
            last_sep_glued: false,
            word_since_sep: false,
            word_after_code: false,
            word_had_cancel: false,
            word_had_modifier_cancel: false,
            sentence_vi: false,
            context_hint: ContextHint::Auto,
            sentence_break_pending: false,
            token_url: false,
            token_len: 0,
            token_has_word: false,
            token_first: '\0',
            token_prev: '\0',
            token_prev2: '\0',
            token_dot: false,
            word_in_url: false,
        }
    }

    pub fn clear(&mut self) {
        self.raw_chars.clear();
        self.key_log.clear();
        self.word_raw.clear();
        self.prev_rendered.clear();
        self.prev_inner_render.clear();
        self.last_valid_raw_len = 0;
        self.last_valid_coda_start = 0;
        self.last_valid_out.clear();
        self.diff_committed.clear();
        self.diff_suffix.clear();
        self.scratch_optimistic.clear();
        self.scratch_display.clear();
        // Clear snapshots
        for s in &mut self.snapshots {
            *s = None;
        }
        self.snapshot_count = 0;
        self.word_raw_overflowed = false;
        self.english_sticky = false;
        self.english_fallback = None;
        // Clear committed-word history
        for w in &mut self.edit_history {
            *w = None;
        }
        self.edit_history_start = 0;
        self.edit_history_len = 0;
        self.last_edit_caret_back = 0;
        // The separator history outlives a commit (the next separator is
        // judged against the char before it), so only a full reset drops it.
        self.word_since_sep = false;
        self.word_after_code = false;
        self.word_had_cancel = false;
        self.word_had_modifier_cancel = false;
        self.word_in_url = false;
    }

    /// Drop the sentence context: a full reset, after which the engine no
    /// longer tracks the text the context described. An ordinary word commit
    /// and the ring-preserving commit paths keep it.
    pub fn clear_sentence(&mut self) {
        self.sentence_vi = false;
        self.context_hint = ContextHint::Auto;
        self.sentence_break_pending = false;
    }

    /// The sentence is over: the next word starts a fresh one.
    pub(crate) fn end_sentence(&mut self) {
        self.sentence_vi = false;
        self.context_hint = ContextHint::Auto;
        self.sentence_break_pending = false;
    }

    /// Drop the separator history (`last_sep` / glued state) and the current
    /// token — full reset paths only, never an ordinary word commit.
    pub fn clear_separator(&mut self) {
        self.last_sep = '\0';
        self.last_sep_glued = false;
        self.token_url = false;
        self.token_len = 0;
        self.token_has_word = false;
        self.token_first = '\0';
        self.token_prev = '\0';
        self.token_prev2 = '\0';
        self.token_dot = false;
    }

    /// Push a committed word onto the ring (oldest evicted when full).
    /// The new entry starts with no separators after it; the host reports the
    /// ones typed after the word through `note_separator`.
    pub fn push_committed(&mut self, raw: CharVec<24>, rendered: OutBuffer) {
        // The word now on screen decides the sentence language: a Vietnamese
        // render keeps the rest of the sentence Vietnamese.
        if is_vietnamese_evidence(&raw, &rendered) {
            self.sentence_vi = true;
        }
        if self.edit_history_len == self.edit_history.len() {
            self.edit_history[self.edit_history_start] = None;
            self.edit_history_start = (self.edit_history_start + 1) % self.edit_history.len();
            self.edit_history_len -= 1;
        }
        let idx = (self.edit_history_start + self.edit_history_len) % self.edit_history.len();
        self.edit_history[idx] = Some(CommittedWord {
            raw,
            rendered,
            gap: 0,
            sep: '\0',
            editable: true,
            had_cancel: false,
            ends_sentence: false,
        });
        self.edit_history_len += 1;
        self.word_since_sep = true;
        // The token holds a word now, so its next separator char is not the
        // token's first character.
        self.token_has_word = true;
    }

    /// Resolve a pending `.` sentence boundary: the dot ends the sentence
    /// unless the new word starts right after it ("a.b" — the two halves of a
    /// code identifier, not two sentences).
    ///
    /// `after_separator` is true when another separator char followed the dot
    /// (`hocj. ` — prose, so a boundary whatever the dot was glued to) and
    /// false when a word starts (`a.b`, `3.5` — the dot is a code/decimal
    /// dot).
    pub(crate) fn resolve_sentence_break(&mut self, after_separator: bool) {
        if !self.sentence_break_pending {
            return;
        }
        self.sentence_break_pending = false;
        if !after_separator && self.last_sep == '.' && self.last_sep_glued {
            return;
        }
        self.end_sentence();
        if let Some(w) = self.committed_mut(0) {
            w.ends_sentence = true;
        }
    }

    /// The current sentence's committed words with their separators, oldest
    /// first — the text a host runs its language recognizer on.
    ///
    /// Rebuilt from the committed-word ring, so a sentence longer than the
    /// ring keeps only its newest words. A sentence-end separator ends the
    /// walk; each word contributes the separators typed after it (`gap` of
    /// them — the last one exact, the earlier ones remembered only as a
    /// count, so spaces stand in).
    pub fn sentence_text(&self) -> StackStr<512> {
        let mut out = StackStr::<512>::new();
        // Steps count back from the newest word, so the first sentence end
        // found is the one that started this sentence.
        let mut first = self.edit_history_len;
        for step in 0..self.edit_history_len {
            if self.committed(step).is_some_and(|w| w.ends_sentence) {
                first = step;
                break;
            }
        }
        for step in (0..first).rev() {
            let Some(word) = self.committed(step) else {
                break;
            };
            let mut ok = out.push_str(&word.rendered);
            for _ in 0..word.gap.saturating_sub(1) {
                ok &= out.push(' ');
            }
            if word.gap > 0 {
                ok &= out.push(word.sep);
            }
            if !ok {
                break;
            }
        }
        out
    }

    /// Ring slot of the entry at `step` (0 = newest, growing older).
    #[inline]
    fn ring_index(&self, step: usize) -> Option<usize> {
        if step >= self.edit_history_len {
            return None;
        }
        Some((self.edit_history_start + self.edit_history_len - 1 - step) % self.edit_history.len())
    }

    /// Borrow the committed word at `step` (0 = newest).
    #[inline]
    pub(crate) fn committed(&self, step: usize) -> Option<&CommittedWord> {
        let idx = self.ring_index(step)?;
        self.edit_history[idx].as_ref()
    }

    /// Mutably borrow the committed word at `step` (0 = newest).
    #[inline]
    pub(crate) fn committed_mut(&mut self, step: usize) -> Option<&mut CommittedWord> {
        let idx = self.ring_index(step)?;
        self.edit_history[idx].as_mut()
    }

    /// Separator chars on screen right after the newest committed word.
    #[inline]
    pub(crate) fn trailing_gap(&self) -> usize {
        self.committed(0).map_or(0, |w| w.gap)
    }

    /// A separator char (space / punctuation / Enter) was typed right after
    /// the newest committed word. No-op while nothing is committed.
    ///
    /// The char itself is remembered: a code char glued to the text before it
    /// (`foo_bar`, `a.b`) marks the next word as a code identifier, and the
    /// retroactive English fix needs the exact separator between two words.
    #[inline]
    pub(crate) fn note_separator(&mut self, ch: char) {
        // A separator right after a committed word is glued to it (`_` in
        // `foo_bar`); one that follows another separator is glued only when
        // that separator was not a space (`,` `.` in "a,."). A separator
        // after a space is normal punctuation (`(` in "foo (bar").
        let glued =
            self.word_since_sep || !matches!(self.last_sep, ' ' | '\t' | '\n' | '\r' | '\0');
        // Sentence boundary: `!`, `?` and a newline always end the sentence.
        // A `.` is only a boundary when it is not a URL/code dot: inside a
        // token (`github.com/x`, `https://x.y`) it is part of the address, and
        // glued between two letters ("a.b") it is a code identifier. The
        // latter is decided when the next word starts — the `.` alone cannot
        // tell "a.b" from "test. Next".
        if ch == '.' {
            if !self.token_url {
                self.sentence_break_pending = true;
            }
        } else if is_sentence_end(ch) {
            self.end_sentence();
            if let Some(w) = self.committed_mut(0) {
                w.ends_sentence = true;
            }
        } else {
            // Any other separator after the dot proves it was prose
            // punctuation, not a code/decimal dot.
            self.resolve_sentence_break(true);
        }
        self.last_sep = ch;
        self.last_sep_glued = glued;
        self.word_since_sep = false;
        self.note_token_char(ch, glued);
        if let Some(w) = self.committed_mut(0) {
            w.gap += 1;
            w.sep = ch;
        }
    }

    /// Track the current whitespace-delimited token so a word inside a URL,
    /// path or code token can show its literal keystrokes (`/usr/share`,
    /// `https://github.com/x/...`, `github.com/x`). Whitespace starts a new
    /// token; every other separator char grows the current one.
    fn note_token_char(&mut self, ch: char, glued: bool) {
        if matches!(ch, ' ' | '\t' | '\n' | '\r') {
            self.token_url = false;
            self.token_len = 0;
            self.token_has_word = false;
            self.token_first = '\0';
            self.token_prev = '\0';
            self.token_prev2 = '\0';
            self.token_dot = false;
            return;
        }
        let first = self.token_len == 0 && !self.token_has_word;
        if first {
            self.token_first = ch;
        }
        if ch == '/' {
            // A leading `/` ("/usr"), `~/` or `./`, a `.` before a `/`
            // ("github.com/x"), and the `://` of a scheme.
            if first
                || (self.token_len == 1 && matches!(self.token_first, '~' | '.'))
                || self.token_dot
                || (self.token_prev == '/' && self.token_prev2 == ':')
            {
                self.token_url = true;
            }
        } else if (first && matches!(ch, '`' | '@' | '_' | '=' | '\\' | '$'))
            || (glued && crate::diff::is_code_glue_char(ch))
        {
            // A code marker opening the token ("`/usr/...") or a code char
            // glued to its text ("signed-by=", "a.b") makes the rest of the
            // token an identifier.
            self.token_url = true;
        }
        if ch == '.' {
            self.token_dot = true;
        }
        self.token_prev2 = self.token_prev;
        self.token_prev = ch;
        self.token_len = self.token_len.saturating_add(1);
    }

    /// A separator char right after the newest committed word was deleted.
    /// Returns false when there is no separator left to account for.
    #[inline]
    pub(crate) fn remove_separator(&mut self) -> bool {
        match self.committed_mut(0) {
            Some(w) if w.gap > 0 => {
                w.gap -= 1;
                true
            }
            _ => false,
        }
    }

    /// Remove the committed word at `step` (0 = newest) and shift the ring so
    /// the remaining entries keep their order. The caller merges the removed
    /// entry's gap into the older neighbor (the separators that surrounded it
    /// become adjacent on screen).
    pub(crate) fn remove_committed(&mut self, step: usize) -> Option<CommittedWord> {
        if step >= self.edit_history_len {
            return None;
        }
        let ring_len = self.edit_history.len();
        let mut removed = None;
        let mut kept: [Option<CommittedWord>; 8] = [const { None }; 8];
        let mut n = 0usize;
        // Oldest → newest so `kept` keeps the ring order.
        for s in (0..self.edit_history_len).rev() {
            let idx = (self.edit_history_start + self.edit_history_len - 1 - s) % ring_len;
            let word = self.edit_history[idx].take();
            if s == step {
                removed = word;
            } else if let Some(word) = word {
                kept[n] = Some(word);
                n += 1;
            }
        }
        for (i, word) in kept.into_iter().take(n).enumerate() {
            self.edit_history[(self.edit_history_start + i) % ring_len] = word;
        }
        for i in n..ring_len {
            self.edit_history[(self.edit_history_start + i) % ring_len] = None;
        }
        self.edit_history_len = n;
        removed
    }

    /// Borrow the newest committed word that has raw keystrokes, skipping
    /// empty entries (a commit with nothing composing records nothing, so
    /// these only come from defensive paths).
    ///
    /// Read by the sentence-context English rule at word commit: the
    /// previous word must be on screen as plain English text.
    pub fn newest_committed(&self) -> Option<&CommittedWord> {
        for step in 0..self.edit_history_len {
            let idx = (self.edit_history_start + self.edit_history_len - 1 - step)
                % self.edit_history.len();
            if let Some(w) = self.edit_history[idx].as_ref() {
                if !w.raw.is_empty() {
                    return Some(w);
                }
            }
        }
        None
    }

    /// Push a snapshot of the current composing state.
    /// Called after each successful feed_diff_core.
    #[inline]
    pub fn push_snapshot(&mut self, snap: ComposingSnapshot) {
        if self.snapshot_count < 24 {
            self.snapshots[self.snapshot_count] = Some(snap);
            self.snapshot_count += 1;
        }
    }

    /// Pop and return the last snapshot, or None if empty.
    #[inline]
    pub fn pop_snapshot(&mut self) -> Option<ComposingSnapshot> {
        if self.snapshot_count == 0 {
            return None;
        }
        self.snapshot_count -= 1;
        self.snapshots[self.snapshot_count].take()
    }
}
