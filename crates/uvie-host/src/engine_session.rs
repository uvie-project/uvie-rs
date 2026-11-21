//! Thin wrapper over `uvie::UltraFastViEngine` exposing the diff API
//! (`feed`/`backspace`/`commit` -> backspaces + replacement suffix) that the
//! host's text injector consumes, plus the engine's option set.

use uvie::diff::Diffable;
use uvie::{InputMethod, UltraFastViEngine};

/// The engine feature flags the host can toggle. Mirrors the knobs uvie-mac
/// exposes in its settings panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EngineOptions {
    /// `j`->`gi`, `f`->`ph`, `w`->`qu` expansion on word start.
    pub quick_start: bool,
    /// Double-letter shorthand after first char (`cc`->`ch`, `gg`->`gi`, ...).
    pub quick_telex: bool,
    /// Modern orthography tone placement (`oà`, `uý` style off by default).
    pub modern_orthography: bool,
    /// Allow non-standard coda consonants.
    pub relaxed_coda: bool,
    /// English-dictionary override: real English words pass through untouched.
    pub english_override: bool,
}

/// One live engine instance. Host code creates one per text-input session
/// (uvie-mac keeps a single engine plus per-app remembered state).
pub struct EngineSession {
    engine: UltraFastViEngine,
}

impl Default for EngineSession {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineSession {
    pub fn new() -> Self {
        Self {
            engine: UltraFastViEngine::new(),
        }
    }

    pub fn with_options(method: InputMethod, opts: EngineOptions) -> Self {
        let mut s = Self::new();
        s.set_input_method(method);
        s.apply_options(opts);
        s
    }

    pub fn set_input_method(&mut self, method: InputMethod) {
        // Settings hot-reload calls this on every mtime tick. Re-applying the
        // same method must not drop a live composition (and the committed-word
        // ring) mid-word.
        if self.engine.input_method() == method {
            return;
        }
        self.engine.set_input_method(method);
        self.engine.reset_diff();
    }

    pub fn input_method(&self) -> InputMethod {
        self.engine.input_method()
    }

    pub fn apply_options(&mut self, opts: EngineOptions) {
        self.engine.set_quick_start(opts.quick_start);
        self.engine.set_quick_telex(opts.quick_telex);
        self.engine.set_modern_orthography(opts.modern_orthography);
        self.engine.set_relaxed_coda(opts.relaxed_coda);
        self.engine.set_english_override(opts.english_override);
    }

    /// Allow or forbid rewrites that touch already-committed text (retro
    /// English restore, contraction spans). Hosts whose committed text is
    /// immutable (preedit shells like ibus/fcitx) pass false.
    pub fn set_cross_word_rewrite(&mut self, enabled: bool) {
        self.engine.set_cross_word_rewrite(enabled);
    }

    /// Feed one ASCII char. Returns `(backspaces, new_suffix)` - the host must
    /// erase `backspaces` chars then type `new_suffix`.
    pub fn feed(&mut self, ch: char) -> (usize, String) {
        let (bs, s) = self.engine.feed_diff(ch);
        (bs, s.to_owned())
    }

    /// User pressed Backspace. Returns `(backspaces, new_suffix)`.
    pub fn backspace(&mut self) -> (usize, String) {
        let (bs, s) = self.engine.backspace_diff();
        (bs, s.to_owned())
    }

    /// Word boundary reached. Returns `(backspaces, suffix)` - usually `(0, "")`.
    pub fn commit(&mut self) -> (usize, String) {
        let (bs, s) = self.engine.commit_diff();
        (bs, s.to_owned())
    }

    /// Word boundary reached at the separator key `boundary` (the char that
    /// ends the word: space, punctuation, Enter). The char decides the
    /// code-identifier rule (`feed_diff`, `raw_chars`, `UVIE_RS_DIR`).
    /// Returns `(backspaces, suffix)`.
    pub fn commit_with(&mut self, boundary: char) -> (usize, String) {
        let (bs, s) = self.engine.commit_with_boundary_diff(boundary);
        (bs, s.to_owned())
    }

    /// LabanKey-style post-commit edit: user moved the caret back onto a
    /// committed word and typed `ch`. `caret_back` is the caret distance back
    /// to the end of the newest committed word.
    /// Returns `(backspaces + 1, forward_deletes, new_suffix)` when handled.
    pub fn edit_at(&mut self, caret_back: usize, ch: char) -> Option<(usize, usize, String)> {
        self.engine
            .edit_at_caret_diff(caret_back, ch)
            .map(|(bs, fwd, s)| (bs, fwd, s.to_owned()))
    }

    /// Backspace with the caret on a committed word's end: the app deletes one
    /// visible char natively and the engine only shrinks that word's tracked
    /// text so it stays editable. Returns true when the engine tracked it.
    pub fn delete_char_at(&mut self, caret_back: usize) -> bool {
        self.engine.delete_char_at_caret(caret_back)
    }

    /// Caret offset the dispatcher must adopt after the last successful edit.
    pub fn last_caret_back_after_edit(&self) -> isize {
        self.engine.last_caret_back_after_edit()
    }

    /// A separator char (space / punctuation / Enter) was typed right after
    /// the newest committed word. A code char glued to the text before it
    /// (`_` in `foo_bar`, `.` in `a.b`) marks the next word as code.
    pub fn note_separator(&mut self, ch: char) {
        self.engine.note_separator(ch);
    }

    /// A separator char right after the newest committed word was deleted.
    pub fn remove_separator(&mut self) -> bool {
        self.engine.remove_separator()
    }

    /// The separator at the END of the newest word's trailing gap was deleted
    /// (Backspace with the caret at the end of the gap): exactly undo what its
    /// `note_separator` did instead of degrading the gap to "unknown".
    pub fn remove_last_separator(&mut self) -> bool {
        self.engine.remove_last_separator()
    }

    /// Separator chars currently on screen right after the newest word.
    pub fn trailing_gap(&self) -> usize {
        self.engine.trailing_gap()
    }

    /// The current sentence's committed words with their separators - the
    /// text a host hands a language recognizer.
    pub fn sentence_text(&self) -> String {
        self.engine.sentence_text().to_string()
    }

    /// A committed word is on record: the caret offsets are measured against
    /// it. Without one, a separator at the caret carries no geometry.
    pub fn has_committed(&self) -> bool {
        self.engine.has_committed_word()
    }

    /// Drop all composing + committed state (app switch, caret jump, reset).
    /// Must use the diff-aware reset: the plain core `clear()` leaves the
    /// committed-word ring intact, so a later keystroke could still "edit"
    /// a word the host has already forgotten.
    pub fn reset(&mut self) {
        self.engine.reset_diff();
    }

    /// True while the engine owns text on screen. Must use the diff-aware
    /// predicate: the core `is_composing()` only looks at the live raw buffer,
    /// while the diff state can still own a committed prefix or an English
    /// dictionary word after that buffer was cleared.
    pub fn is_composing(&self) -> bool {
        self.engine.is_composing_diff()
    }

    /// Text the engine has already auto-committed (V-C-V splits).
    pub fn committed_text(&self) -> String {
        self.engine.committed_text_diff().to_owned()
    }

    /// The rendered (post-conversion) text of the current composing word.
    /// Macro matching needs committed + composing on-screen text, same as
    /// uvie-mac's `getCurrentText()`.
    pub fn current_output(&self) -> String {
        self.engine.current_output_diff().to_string()
    }

    /// Raw keystrokes of the whole current word (across V-C-V splits, tone
    /// cancels and dictionary commits). Empty when the word outgrew the
    /// engine's raw buffer - the host must not restore it then.
    pub fn raw_chars(&self) -> String {
        self.engine.word_raw_string().to_string()
    }
}
