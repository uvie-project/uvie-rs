//! uvie-host — platform-independent host policy for UVie input methods.
//!
//! Mirrors the portable half of uvie-mac's `Core/` + `Features/` layers: it
//! owns the `uvie` engine session, decides how each keystroke is dispatched
//! (`Pass` vs `Consume` plus the `InputAction` plan), and holds the macros and
//! per-app language memory the platform front ends share.
//!
//! The crate is intentionally free of OS API calls so all dispatch and typing
//! behavior is unit-testable on any platform. Platform front ends keep the
//! parts that cannot be portable: the event source (CGEventTap, WH_KEYBOARD_LL,
//! evdev), the injection strategy (CGEvent, SendInput, uinput), app identity,
//! accessibility injection, global hotkeys and layout monitoring.

pub mod dispatcher;
pub mod engine_session;
pub mod keys;
pub mod macros;
pub mod memory;

pub use dispatcher::{Context, Dispatch, Dispatcher, InputAction, InputLanguage};
pub use engine_session::{EngineOptions, EngineSession};
pub use keys::{KeyEvent, KeyKind};
pub use macros::{MacroEntry, MacroTable};
pub use memory::{LanguageMemory, LanguagePref};
pub use uvie::InputMethod;

#[cfg(test)]
mod tests;
