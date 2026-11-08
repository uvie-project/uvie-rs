//! Auto-capitalize sentence state machine.
//!
//! Ported from uvie-mac's `EventTap` (`isAtSentenceStart`,
//! `savedIsAtSentenceStart`, `sentenceStartMemory`). The state is about the
//! *sentence*, not the input method: it keeps running in English mode so
//! switching back to Vietnamese mid-word does not capitalize the next letter.

use std::collections::HashMap;

/// Tracks whether the next typed letter starts a new sentence.
///
/// The per-app memory is session-scoped (not persisted): a restart resets to
/// the safe `true` default.
#[derive(Debug)]
pub struct SentenceState {
    at_start: bool,
    /// `at_start` as it was before a pointer-down reset. If an app switch
    /// follows the click, the state belongs to the app being left and is
    /// restored; if no switch follows, the next key press discards it.
    saved: Option<bool>,
    /// app identifier -> `at_start` when that app lost focus.
    memory: HashMap<String, bool>,
    /// The app whose state currently lives in `at_start`.
    memory_app: String,
}

impl Default for SentenceState {
    fn default() -> Self {
        Self::new()
    }
}

impl SentenceState {
    pub fn new() -> Self {
        Self {
            at_start: true,
            saved: None,
            memory: HashMap::new(),
            memory_app: String::new(),
        }
    }

    pub fn is_at_start(&self) -> bool {
        self.at_start
    }

    /// Test/host override. Production code only changes the state through
    /// typing, Enter and app switches.
    pub fn set_at_start(&mut self, value: bool) {
        self.at_start = value;
    }

    pub fn saved(&self) -> Option<bool> {
        self.saved
    }

    pub fn memory_app(&self) -> &str {
        &self.memory_app
    }

    /// Uppercase `c` when auto-capitalize is enabled and the caret is at a
    /// sentence start. Only alphabetic characters consume the sentence-start
    /// window; spaces and punctuation leave it armed (a leading space does not
    /// end the sentence).
    pub fn apply_capitalize(&mut self, enabled: bool, c: char) -> char {
        if !enabled || !self.at_start || !c.is_alphabetic() {
            return c;
        }
        self.at_start = false;
        c.to_uppercase().next().unwrap_or(c)
    }

    /// Feed the character that was typed (or passed through) into the state
    /// machine. `.` `!` `?` start a new sentence; letters and digits end the
    /// current one; everything else leaves the state unchanged.
    pub fn update_after(&mut self, c: char) {
        if matches!(c, '.' | '!' | '?') {
            self.at_start = true;
        } else if c.is_alphabetic() || c.is_numeric() {
            self.at_start = false;
        }
    }

    /// Return/Enter starts a new sentence. Tab and Escape do not.
    pub fn on_enter(&mut self) {
        self.at_start = true;
    }

    /// A pointer-down starts a new editing session: remember the current
    /// state (it belongs to the app being left if this click switches apps)
    /// and default to a sentence start for the new text field.
    pub fn on_pointer_down(&mut self) {
        self.saved = Some(self.at_start);
        self.at_start = true;
    }

    /// Any key press discards the pointer-down save: reaching a key press
    /// without an app switch means the click was a cursor reposition inside
    /// the current app, so the reset to `true` stays.
    pub fn on_key_down(&mut self) {
        self.saved = None;
    }

    /// Swap the sentence state across an app activation. A click-saved value
    /// is restored first (it reflects the leaving app's pre-click state), the
    /// leaving app's state is filed, and the entering app's remembered state
    /// is loaded. An app with no memory yet keeps the current state.
    pub fn on_app_switch(&mut self, app: &str) {
        if let Some(saved) = self.saved.take() {
            self.at_start = saved;
        }
        let old = std::mem::replace(&mut self.memory_app, app.to_string());
        if old != app {
            if !old.is_empty() {
                self.memory.insert(old, self.at_start);
            }
            if let Some(&remembered) = self.memory.get(app) {
                self.at_start = remembered;
            }
        }
    }
}
