//! Keystroke dispatcher - the platform-independent half of the keyboard
//! pipeline, mirroring `EventTap.handle()` in uvie-mac.
//!
//! For every key press the dispatcher decides whether to let it through to
//! the foreground app (`Pass`) or to consume it and synthesize output
//! (`Consume`), and produces the `InputAction` plan the platform injector
//! executes.

use crate::engine_session::EngineSession;
use crate::keys::{KeyEvent, KeyKind, is_feedable_ascii};
use crate::macros::MacroTable;

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
    /// alongside (commit flush).
    Pass(Vec<InputAction>),
    /// Swallow the original key and emit this replacement output instead.
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
    /// When true, auto-capitalize the first letter of a sentence (uvie-mac
    /// "sentence start" behavior - placeholder flag; implementation TODO).
    pub auto_capitalize: bool,
    /// Text macro expansion armed (uvie-mac `macroEnabled`). The table lives
    /// in `self.macros`.
    pub macro_enabled: bool,
    /// Caret distance back to the end of the newest committed word, used to
    /// arm LabanKey-style post-commit editing (`edit_at`). Stays 0 until the
    /// host feeds navigation events (caret tracking).
    caret_back: usize,
}

impl Dispatcher {
    pub fn new(session: EngineSession) -> Self {
        Self {
            session,
            language: InputLanguage::Vietnamese,
            macros: MacroTable::default(),
            auto_capitalize: false,
            macro_enabled: false,
            caret_back: 0,
        }
    }

    /// Toggle Vi/En. Switching clears engine state - mirrors uvie-mac.
    pub fn set_language(&mut self, lang: InputLanguage) {
        if self.language != lang {
            self.language = lang;
            self.session.reset();
            self.caret_back = 0;
        }
    }

    /// Handle one key press. `ctx` carries host state (excluded app, layout).
    pub fn handle(&mut self, key: KeyEvent, ctx: &Context) -> Dispatch {
        // Our own synthesized output echoing back through the event source.
        if key.injected {
            return Dispatch::pass();
        }

        // Shortcut chords (Ctrl+C, Alt+Tab, Win+R, ...) never reach the
        // engine and cancel any composing word, like uvie-mac does.
        if key.ctrl || key.alt || key.win {
            self.session.reset();
            self.caret_back = 0;
            return Dispatch::pass();
        }

        let lang = ctx.language.unwrap_or(self.language);
        if ctx.app_excluded || ctx.non_latin_layout || lang == InputLanguage::English {
            // Disabled paths still keep the engine clean so returning to Vi
            // mode doesn't resurrect stale state.
            self.session.reset();
            self.caret_back = 0;
            return Dispatch::pass();
        }

        match key.kind {
            KeyKind::Char(c) => self.handle_char(c),
            KeyKind::Backspace => self.handle_backspace(),
            KeyKind::Break => self.handle_break(),
            KeyKind::Navigation => {
                // Caret moved: commit the in-flight word (standard IME
                // behavior) and pass the key through. Committed history is
                // preserved so `edit_at` stays usable once caret tracking
                // lands.
                let (bs, suffix) = self.session.commit();
                let mut plan = Vec::new();
                if bs > 0 {
                    plan.push(InputAction::Backspace(bs));
                }
                if !suffix.is_empty() {
                    plan.push(InputAction::Text(suffix));
                }
                Dispatch::Pass(plan)
            }
            KeyKind::Other => Dispatch::pass(),
        }
    }

    fn handle_char(&mut self, c: char) -> Dispatch {
        if !is_feedable_ascii(c) {
            self.session.reset();
            return Dispatch::pass();
        }

        // Word-terminating printable key (space/punctuation): commit first.
        if crate::keys::is_word_break_char(c) {
            // Macro expansion fires on Space only (uvie-mac never expands
            // on punctuation). The space itself is swallowed - the
            // expansion replaces abbreviation+terminator.
            if c == ' ' {
                if let Some(plan) = self.try_expand_macro() {
                    self.caret_back = 0;
                    return Dispatch::Consume(plan);
                }
            }
            let (bs, suffix) = self.session.commit();
            let mut plan = Vec::new();
            if bs > 0 {
                plan.push(InputAction::Backspace(bs));
            }
            if !suffix.is_empty() {
                plan.push(InputAction::Text(suffix));
            }
            self.caret_back = 0;
            return Dispatch::Pass(plan);
        }

        // If the caret had walked back onto a committed word, this char may be
        // a post-commit tone/edit key (LabanKey-style).
        if self.caret_back > 0 {
            if let Some((bs_plus_one, fwd, suffix)) = self.session.edit_at(self.caret_back, c) {
                let mut plan = Vec::new();
                if fwd > 0 {
                    plan.push(InputAction::ForwardDelete(fwd));
                }
                plan.push(InputAction::Backspace(bs_plus_one));
                plan.push(InputAction::Text(suffix));
                self.caret_back = 0;
                return Dispatch::Consume(plan);
            }
            self.caret_back = 0;
        }

        let (bs, suffix) = self.session.feed(c);
        let mut plan = Vec::new();
        if bs > 0 {
            plan.push(InputAction::Backspace(bs));
        }
        plan.push(InputAction::Text(suffix));
        Dispatch::Consume(plan)
    }

    fn handle_backspace(&mut self) -> Dispatch {
        let (bs, suffix) = self.session.backspace();
        if bs == 0 && suffix.is_empty() && !self.session.is_composing() {
            // Nothing buffered - a real Backspace should reach the app.
            return Dispatch::pass();
        }
        let mut plan = Vec::new();
        if bs > 0 {
            plan.push(InputAction::Backspace(bs));
        }
        if !suffix.is_empty() {
            plan.push(InputAction::Text(suffix));
        }
        Dispatch::Consume(plan)
    }

    fn handle_break(&mut self) -> Dispatch {
        // Enter (and other break keys) expand a matching abbreviation,
        // exactly like Space does in uvie-mac - the key is consumed.
        if let Some(plan) = self.try_expand_macro() {
            self.caret_back = 0;
            return Dispatch::Consume(plan);
        }
        let (bs, suffix) = self.session.commit();
        self.caret_back = 0;
        let mut plan = Vec::new();
        if bs > 0 {
            plan.push(InputAction::Backspace(bs));
        }
        if !suffix.is_empty() {
            plan.push(InputAction::Text(suffix));
        }
        Dispatch::Pass(plan)
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
        self.session.reset();
        self.caret_back = 0;
        Some(vec![
            InputAction::Backspace(text.chars().count()),
            InputAction::Text(expansion),
        ])
    }
}
