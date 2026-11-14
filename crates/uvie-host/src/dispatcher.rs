//! Keystroke dispatcher - the platform-independent half of the keyboard
//! pipeline, mirroring `EventTap.handle()` in uvie-mac.
//!
//! For every key press the dispatcher decides whether to let it through to
//! the foreground app (`Pass`) or to consume it and synthesize output
//! (`Consume`), and produces the `InputAction` plan the platform injector
//! executes.
//!
//! Portable behavior ported from uvie-mac:
//! - the composing word is committed on Space, punctuation and Enter, and
//!   dropped on Tab / other caret movement / a pointer-down / an app switch;
//! - the caret distance back to the newest committed word is tracked so
//!   typing a tone/modifier key on an earlier word re-renders it in place
//!   (`edit_at_caret_diff`, LabanKey-style);
//! - Escape restores the raw keystrokes of the composing word (EVKey-style);
//! - non-ASCII keys and keys the engine could not process are passed through
//!   instead of being swallowed;
//! - auto-capitalize keeps a per-app sentence state, in both languages.
//!
//! Deliberate deviations from uvie-mac:
//! - every modifier chord resets the engine (mac only resets for chords that
//!   can change text behind its back); over-resetting is safe;
//! - punctuation is passed through after `commit()` instead of being fed and
//!   re-posted, and the anchor moves to `-1` (the caret is one right of the
//!   committed word), so typing a letter after a period starts a fresh word
//!   instead of editing across the punctuation;
//! - English mode passes characters through instead of re-posting them
//!   synthetically (mac does that to suppress the OS autocomplete popup,
//!   which is a macOS-only concern).

use crate::engine_session::EngineSession;
use crate::keys::{KeyEvent, KeyKind, is_feedable_ascii, is_word_break_char};
use crate::macros::MacroTable;
use crate::sentence::SentenceState;

/// A single action for the host injector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputAction {
    /// Send N Backspace keystrokes.
    Backspace(usize),
    /// Send N forward-Delete keystrokes (mid-word post-commit edits).
    ForwardDelete(usize),
    /// Type UTF-16 text via unicode key events.
    Text(String),
}

/// What the host should do with a key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dispatch {
    /// Let the original key reach the application; optionally emit text
    /// alongside (commit flush, restore-on-Escape). The host must apply the
    /// plan BEFORE delivering the original key.
    Pass(Vec<InputAction>),
    /// Swallow the original key and emit this replacement output instead.
    /// An empty plan means "swallow, inject nothing": the key was absorbed by
    /// the engine's raw buffer and renders nothing yet (the glyph appears on
    /// the next keystroke).
    Consume(Vec<InputAction>),
}

impl Dispatch {
    pub fn pass() -> Self {
        Dispatch::Pass(Vec::new())
    }
}

/// Which input language the user currently wants, mirroring uvie-mac's
/// Vi/En toggle (per-app remembered).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputLanguage {
    Vietnamese,
    English,
}

/// Per-key decision context supplied by the host.
#[derive(Debug, Clone, Copy, Default)]
pub struct Context {
    /// Global Vi/En state for the foreground app.
    pub language: Option<InputLanguage>,
    /// The foreground app is on the user exclusion list.
    pub app_excluded: bool,
    /// The active keyboard layout is non-Latin (IME passes everything).
    pub non_latin_layout: bool,
}

/// The dispatcher owns the engine session and applies user settings.
pub struct Dispatcher {
    pub session: EngineSession,
    pub language: InputLanguage,
    pub macros: MacroTable,
    /// Capitalize the first letter of a sentence.
    pub auto_capitalize: bool,
    /// Text macro expansion armed (uvie-mac `macroEnabled`).
    pub macro_enabled: bool,
    /// LabanKey-style post-commit editing: arrow back onto a committed word
    /// and type a tone key to re-render it. On by default, like uvie-mac.
    pub edit_committed: bool,
    /// Caret distance (screen characters) back to the end of the newest
    /// committed word. 0 = caret at that word's end (edit-armed), negative =
    /// right of it (normal typing position after a commit space), positive =
    /// the caret moved back into earlier text.
    caret_back: isize,
    /// Auto-capitalize sentence state + per-app memory.
    pub sentence: SentenceState,
    /// Upper bound on the invisible-raw-key drain in `handle_backspace`: the
    /// engine's raw buffer holds at most one word (24 keystrokes), so this can
    /// never spin on a stuck engine.
    max_backspace_drain: usize,
}

impl Dispatcher {
    pub fn new(session: EngineSession) -> Self {
        Self {
            session,
            language: InputLanguage::Vietnamese,
            macros: MacroTable::default(),
            auto_capitalize: false,
            macro_enabled: false,
            edit_committed: true,
            caret_back: 0,
            sentence: SentenceState::new(),
            max_backspace_drain: 24,
        }
    }

    /// Caret distance back to the newest committed word (tests / diagnostics).
    pub fn caret_back(&self) -> isize {
        self.caret_back
    }

    pub fn sentence(&self) -> &SentenceState {
        &self.sentence
    }

    /// Toggle Vi/En. Switching clears engine state - mirrors uvie-mac.
    pub fn set_language(&mut self, lang: InputLanguage) {
        if self.language != lang {
            self.language = lang;
            self.reset();
        }
    }

    /// Drop the composing word and the caret anchor. Used by app switches,
    /// language changes, pointer-downs and the host's reset paths.
    pub fn reset(&mut self) {
        self.session.reset();
        self.caret_back = 0;
    }

    /// Focus moved to another app: drop the in-flight composition and swap
    /// the per-app sentence-start state. The host calls this from its focus
    /// watcher before restoring the per-app language.
    pub fn on_app_switch(&mut self, app: &str) {
        self.sentence.on_app_switch(app);
        self.reset();
    }

    /// A mouse/touch pointer went down: the caret may have moved, so drop the
    /// composition and arm the sentence-start save/restore across a possible
    /// app switch.
    pub fn on_pointer_down(&mut self) {
        self.sentence.on_pointer_down();
        self.reset();
    }

    /// The language changed outside `set_language` (menu, tray, hotkey).
    /// Sentence state belongs to the sentence, not the language, so it stays.
    pub fn on_language_changed(&mut self) {
        self.reset();
    }

    /// Handle one key press. `ctx` carries host state (excluded app, layout).
    pub fn handle(&mut self, key: KeyEvent, ctx: &Context) -> Dispatch {
        // Our own synthesized output echoing back through the event source.
        if key.injected {
            return Dispatch::pass();
        }

        if ctx.app_excluded {
            self.reset();
            return Dispatch::pass();
        }

        // Any key press discards a pointer-down sentence-start save: without
        // an app switch in between, the click was a cursor reposition inside
        // the current app.
        self.sentence.on_key_down();

        // Shortcut chords (Ctrl+C, Alt+Tab, Win+R, ...) never reach the
        // engine and cancel any composing word. uvie-mac resets only for the
        // chords that can change text behind its back (selection, paste/cut/
        // undo, dead keys, caret jumps); resetting for every chord is a
        // deliberate, safer deviation.
        if key.ctrl || key.alt || key.win {
            self.reset();
            return Dispatch::pass();
        }

        let lang = ctx.language.unwrap_or(self.language);
        // English mode keeps running before the non-Latin check: the sentence
        // state machine is about the sentence, not the layout (uvie-mac order).
        if lang == InputLanguage::English {
            return self.handle_english(key);
        }

        if ctx.non_latin_layout {
            self.reset();
            return Dispatch::pass();
        }

        match key.kind {
            KeyKind::Char(c) => self.handle_char(c),
            KeyKind::Backspace => self.handle_backspace(),
            KeyKind::Enter => self.handle_enter(),
            // Tab moves focus; the single-line anchor model becomes invalid.
            KeyKind::Tab => {
                self.reset();
                Dispatch::pass()
            }
            KeyKind::Escape => self.handle_escape(),
            KeyKind::Left | KeyKind::Right => self.handle_arrow(key),
            // Up/Down/Home/End/PageUp/PageDown jump the caret somewhere the
            // diff engine cannot track.
            KeyKind::Navigation => {
                self.reset();
                Dispatch::pass()
            }
            KeyKind::ForwardDelete => self.handle_forward_delete(key),
            KeyKind::Break => self.handle_break(),
            KeyKind::Other => Dispatch::pass(),
        }
    }

    /// English mode: the app types the character natively. Auto-capitalize
    /// still applies (it is about the sentence, not the input method), so the
    /// rare capitalized first letter is injected and the key consumed. The
    /// sentence state machine keeps running so returning to Vietnamese
    /// mid-word does not capitalize the next letter.
    fn handle_english(&mut self, key: KeyEvent) -> Dispatch {
        match key.kind {
            KeyKind::Char(c) => {
                let out = self.sentence.apply_capitalize(self.auto_capitalize, c);
                self.sentence.update_after(c);
                if out == c {
                    Dispatch::pass()
                } else {
                    Dispatch::Consume(vec![InputAction::Text(out.to_string())])
                }
            }
            KeyKind::Enter => {
                self.sentence.on_enter();
                Dispatch::pass()
            }
            _ => Dispatch::pass(),
        }
    }

    fn handle_char(&mut self, c: char) -> Dispatch {
        // The engine only understands ASCII. A non-ASCII key (an accented
        // letter from a Latin layout, Cyrillic/Greek/Arabic) is a word
        // boundary it cannot render: commit what is on screen, drop the
        // state, and let the app insert the character natively. Consuming it
        // would silently swallow the keystroke.
        if !is_feedable_ascii(c) {
            return self.handle_non_ascii(c);
        }

        // Space and punctuation terminate the word. They are passed through
        // (the app inserts them natively) after the composing word commits.
        if is_word_break_char(c) {
            return self.handle_word_break(c);
        }

        let transformed = self.sentence.apply_capitalize(self.auto_capitalize, c);

        // Post-commit editing: the caret sits at or inside a committed word
        // and the user types a key - re-enter that word with the key applied
        // and re-render it in place.
        if self.edit_committed && !self.session.is_composing() && self.caret_back >= 0 {
            if let Some((bs, fwd, out)) =
                self.session.edit_at(self.caret_back as usize, transformed)
            {
                self.sentence.update_after(c);
                self.caret_back = self.session.last_caret_back_after_edit();
                let mut plan = Vec::new();
                push_plan(&mut plan, bs, fwd, &out);
                return Dispatch::Consume(plan);
            }
        }
        // Off-boundary typing inside earlier text invalidates the
        // committed-word history. Also runs with `edit_committed` off, so a
        // stale anchor cannot fire an edit when the setting is re-enabled.
        if !self.session.is_composing() && self.caret_back > 0 {
            self.reset();
        }
        // Right of the anchor the caret is only tracked through the trailing
        // separators; anywhere else is an unseen caret jump.
        if !self.session.is_composing()
            && self.caret_back < 0
            && self.caret_back != -(self.session.trailing_gap() as isize)
        {
            self.reset();
        }

        let (bs, out) = self.session.feed(transformed);
        self.sentence.update_after(c);
        // Typing right of the anchor starts a fresh word after it. Track the
        // RENDERED delta, not the keystroke: transforms replace existing
        // chars and consumed modifier keys produce no output.
        if self.caret_back < 0 {
            self.caret_back -= out.chars().count() as isize - bs as isize;
        }

        // Defense in depth: an empty diff with no composing state means there
        // is nothing to inject - consuming the key would silently swallow it.
        if bs == 0 && out.is_empty() && !self.session.is_composing() {
            return Dispatch::pass();
        }

        let mut plan = Vec::new();
        push_plan(&mut plan, bs, 0, &out);
        Dispatch::Consume(plan)
    }

    /// Non-ASCII key: commit the composing word (the rendered text stays on
    /// screen), drop all state, and pass the key through.
    fn handle_non_ascii(&mut self, c: char) -> Dispatch {
        let mut plan = Vec::new();
        if self.session.is_composing() {
            let (bs, out) = self.session.commit();
            push_plan(&mut plan, bs, 0, &out);
        }
        self.reset();
        self.sentence.update_after(c);
        Dispatch::Pass(plan)
    }

    /// Space / punctuation: expand a macro on Space, commit the word, and let
    /// the app insert the character. The caret anchor moves one character
    /// right (or is dropped when the character lands inside committed text).
    fn handle_word_break(&mut self, c: char) -> Dispatch {
        if c == ' ' {
            if let Some(plan) = self.try_expand_macro() {
                return Dispatch::Consume(plan);
            }
        }
        let was_composing = self.session.is_composing();
        let (bs, out) = self.session.commit();
        let mut plan = Vec::new();
        push_plan(&mut plan, bs, 0, &out);
        self.after_word_break(was_composing);
        self.sentence.update_after(c);
        Dispatch::Pass(plan)
    }

    /// Enter: expand a macro, commit, move the anchor, and start a sentence.
    fn handle_enter(&mut self) -> Dispatch {
        if let Some(plan) = self.try_expand_macro() {
            self.sentence.on_enter();
            return Dispatch::Consume(plan);
        }
        let was_composing = self.session.is_composing();
        let (bs, out) = self.session.commit();
        let mut plan = Vec::new();
        push_plan(&mut plan, bs, 0, &out);
        self.after_word_break(was_composing);
        self.sentence.on_enter();
        Dispatch::Pass(plan)
    }

    /// Shared caret bookkeeping after a word-terminating key was passed
    /// through: the app inserts the separator natively, at the caret, so the
    /// newest committed word grows its on-screen gap and the caret moves one
    /// right of it. When the caret is not at the end of the tracked separators
    /// (inside committed text, or an untracked spot) the history is dropped.
    fn after_word_break(&mut self, was_composing: bool) {
        if was_composing {
            // The commit just recorded the word; the separator lands right
            // after it.
            self.caret_back = 0;
            self.session.note_separator();
            self.caret_back = -1;
        } else if self.caret_back <= 0 && self.caret_back == -(self.session.trailing_gap() as isize)
        {
            self.session.note_separator();
            self.caret_back -= 1;
        } else {
            self.reset();
        }
    }

    /// Escape: while composing, erase the rendered word and type the literal
    /// raw keystrokes back (EVKey-style "restore English"), consuming the key.
    /// When the render already equals the raw input there is nothing to
    /// restore, so Escape keeps its normal role and passes through.
    fn handle_escape(&mut self) -> Dispatch {
        if self.session.is_composing() {
            let out = self.session.current_output();
            let raw = self.session.raw_chars();
            // An empty raw means the word outgrew the engine's raw buffer
            // (its prefix was committed mid-word): restoring would delete
            // text the raw no longer covers, so cancel instead.
            if !raw.is_empty() && raw != out {
                let mut plan = Vec::new();
                push_plan(&mut plan, out.chars().count(), 0, &raw);
                self.reset();
                return Dispatch::Consume(plan);
            }
        }
        self.reset();
        Dispatch::pass()
    }

    /// Plain Left/Right: commit a live composition (it stays on screen) and
    /// re-root the anchor at the caret. Shift+arrow is a selection, which the
    /// diff engine cannot observe, so it resets instead.
    fn handle_arrow(&mut self, key: KeyEvent) -> Dispatch {
        if key.shift {
            self.reset();
            return Dispatch::pass();
        }
        let left = matches!(key.kind, KeyKind::Left);
        if self.session.is_composing() {
            let (bs, out) = self.session.commit();
            let mut plan = Vec::new();
            push_plan(&mut plan, bs, 0, &out);
            self.caret_back = if left { 1 } else { -1 };
            Dispatch::Pass(plan)
        } else {
            self.caret_back += if left { 1 } else { -1 };
            Dispatch::pass()
        }
    }

    /// Forward Delete removes the character right of the caret; the caret
    /// itself does not move and no boundary character is inserted. Only a live
    /// composition is committed (an idle commit records nothing).
    /// Shift+Delete is a cut, which the engine cannot observe, so it resets.
    fn handle_forward_delete(&mut self, key: KeyEvent) -> Dispatch {
        if key.shift {
            self.reset();
            return Dispatch::pass();
        }
        let mut plan = Vec::new();
        if self.session.is_composing() {
            let (bs, out) = self.session.commit();
            push_plan(&mut plan, bs, 0, &out);
            // The caret sits at the just-committed word's end.
            self.caret_back = 0;
        }
        // Forward Delete never moves the caret. Between the newest word's end
        // and the end of its trailing separators it removes one separator
        // (the gap shrinks, the distance back to the word is unchanged). At
        // the end of the gap it removes untracked text, which leaves the ring
        // intact. Inside committed text the geometry breaks: reset.
        if self.caret_back > 0 {
            self.reset();
        } else if ((-self.caret_back) as usize) < self.session.trailing_gap() {
            self.session.remove_separator();
        }
        Dispatch::Pass(plan)
    }

    /// Other word-terminating keys (Help, ...): commit and pass, without the
    /// Enter sentence reset.
    fn handle_break(&mut self) -> Dispatch {
        if let Some(plan) = self.try_expand_macro() {
            return Dispatch::Consume(plan);
        }
        let was_composing = self.session.is_composing();
        let (bs, out) = self.session.commit();
        let mut plan = Vec::new();
        push_plan(&mut plan, bs, 0, &out);
        self.after_word_break(was_composing);
        Dispatch::Pass(plan)
    }

    fn handle_backspace(&mut self) -> Dispatch {
        // Idle: the caret is either on a committed word's end (re-open the
        // word and undo its last keystroke) or inside the separators right of
        // it (delete one of them).
        if !self.session.is_composing() {
            return self.handle_idle_backspace();
        }

        let (mut bs, mut out) = self.session.backspace();
        // A trailing raw key that renders nothing (a key the engine kept in
        // its raw buffer, e.g. "aow" -> screen "ao", "ddf" -> "đ") makes the
        // pop report an empty diff. Consuming the Backspace then deletes
        // nothing, so the key looks dead. Keep popping invisible raw keys
        // until the diff touches the screen (or the word is gone).
        if bs == 0 && out.is_empty() && self.session.is_composing() {
            for _ in 0..self.max_backspace_drain {
                let (b, o) = self.session.backspace();
                bs = b;
                out = o;
                if bs > 0 || !out.is_empty() || !self.session.is_composing() {
                    break;
                }
            }
        }

        if bs == 0 && out.is_empty() && !self.session.is_composing() {
            // Not composing (or the drain emptied the word) - the OS deletes
            // the character left of the caret and the caret moves one left.
            if self.caret_back >= 0 {
                self.reset();
            } else {
                self.session.remove_separator();
                self.caret_back += 1;
            }
            return Dispatch::pass();
        }

        // The engine consumed the backspace: the caret moved by the rendered
        // delta (inserted suffix chars minus erased chars).
        self.caret_back -= out.chars().count() as isize - bs as isize;
        let mut plan = Vec::new();
        push_plan(&mut plan, bs, 0, &out);
        Dispatch::Consume(plan)
    }

    /// Backspace with nothing composing: undo the committed word ending at
    /// the caret, or drop one of the separators right of it. Anything else is
    /// an untracked caret and resets the history (the OS deletes the char).
    fn handle_idle_backspace(&mut self) -> Dispatch {
        if self.caret_back >= 0 {
            if self.edit_committed {
                if let Some((bs, out)) = self.session.backspace_at(self.caret_back as usize) {
                    self.caret_back = self.session.last_caret_back_after_edit();
                    let mut plan = Vec::new();
                    push_plan(&mut plan, bs, 0, &out);
                    return Dispatch::Consume(plan);
                }
            }
            self.reset();
            return Dispatch::pass();
        }
        if self.session.remove_separator() {
            self.caret_back += 1;
        } else {
            self.reset();
        }
        Dispatch::pass()
    }

    /// If the on-screen word (committed prefix + composing) is a macro
    /// trigger, produce the expansion plan: erase it, type the expansion.
    /// Mirrors `applyMacroExpansion` in uvie-mac's EventTap.
    fn try_expand_macro(&mut self) -> Option<Vec<InputAction>> {
        if !self.macro_enabled {
            return None;
        }
        let committed = self.session.committed_text();
        let composing = self.session.current_output();
        if committed.is_empty() && composing.is_empty() {
            return None;
        }
        let text = format!("{committed}{composing}");
        let expansion = self.macros.lookup(&text)?.to_owned();
        self.reset();
        Some(vec![
            InputAction::Backspace(text.chars().count()),
            InputAction::Text(expansion),
        ])
    }
}

/// Append `forward deletes -> backspaces -> text` to a plan. The order is the
/// one the mid-word edit requires: remove the old tail first, then erase the
/// prefix left of the caret, then type the re-rendered word.
fn push_plan(plan: &mut Vec<InputAction>, bs: usize, fwd: usize, out: &str) {
    if fwd > 0 {
        plan.push(InputAction::ForwardDelete(fwd));
    }
    if bs > 0 {
        plan.push(InputAction::Backspace(bs));
    }
    if !out.is_empty() {
        plan.push(InputAction::Text(out.to_string()));
    }
}
