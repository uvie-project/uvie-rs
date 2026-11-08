//! Headless tests in the same spirit as uvie-mac's EngineTypingTests /
//! DispatcherTests / HelperTests: drive the real engine through the
//! dispatcher and assert the reconstructed on-screen text.

use crate::dispatcher::{Context, Dispatch, Dispatcher, InputAction, InputLanguage};
use crate::engine_session::{EngineOptions, EngineSession};
use crate::keys::{KeyEvent, KeyKind};
use crate::memory::{LanguageMemory, LanguagePref};
use uvie::InputMethod;

fn dispatcher() -> Dispatcher {
    Dispatcher::new(EngineSession::with_options(
        InputMethod::Telex,
        EngineOptions {
            english_override: true,
            ..Default::default()
        },
    ))
}

fn apply(screen: &mut String, plan: &[InputAction]) {
    for action in plan {
        match action {
            InputAction::Backspace(n) => {
                for _ in 0..*n {
                    screen.pop();
                }
            }
            InputAction::ForwardDelete(_) => {}
            InputAction::Text(t) => screen.push_str(t),
        }
    }
}

/// Replay `text` like a real event source: consumed plans mutate the screen,
/// passed keys append themselves.
fn replay(d: &mut Dispatcher, text: &str) -> String {
    let mut screen = String::new();
    for ch in text.chars() {
        match d.handle(KeyEvent::char(ch), &Context::default()) {
            Dispatch::Pass(plan) => {
                apply(&mut screen, &plan);
                screen.push(ch);
            }
            Dispatch::Consume(plan) => apply(&mut screen, &plan),
        }
    }
    screen
}

#[test]
fn telex_tones_and_modifiers() {
    let mut d = dispatcher();
    // Classic Telex: 's'->sắc, 'f'->huyền, 'r'->hỏi, 'x'->ngã, 'j'->nặng,
    // double vowels for circumflex, 'w' for breve/horn.
    assert_eq!(replay(&mut d, "vieetj "), "việt ");
    assert_eq!(replay(&mut d, "xin chaof "), "xin chào ");
    assert_eq!(replay(&mut d, "dduowjc "), "được ");
    assert_eq!(replay(&mut d, "tieengs "), "tiếng ");
}

#[test]
fn backspace_walks_back_through_composition() {
    let mut d = dispatcher();
    let mut screen = String::new();
    for ch in "vie".chars() {
        match d.handle(KeyEvent::char(ch), &Context::default()) {
            Dispatch::Consume(plan) | Dispatch::Pass(plan) => apply(&mut screen, &plan),
        }
    }
    // Backspace removes the last raw key; engine re-renders the remainder.
    match d.handle(KeyEvent::backspace(), &Context::default()) {
        Dispatch::Consume(plan) => apply(&mut screen, &plan),
        Dispatch::Pass(plan) => {
            apply(&mut screen, &plan);
            screen.pop();
        }
    }
    assert_eq!(screen, "vi");
}

#[test]
fn excluded_app_passes_keys_untouched() {
    let mut d = dispatcher();
    let ctx = Context {
        app_excluded: true,
        ..Default::default()
    };
    for ch in "viets".chars() {
        assert!(matches!(
            d.handle(KeyEvent::char(ch), &ctx),
            Dispatch::Pass(_)
        ));
    }
}

#[test]
fn english_mode_passes_keys_untouched() {
    let mut d = dispatcher();
    d.set_language(InputLanguage::English);
    for ch in "viets".chars() {
        assert!(matches!(
            d.handle(KeyEvent::char(ch), &Context::default()),
            Dispatch::Pass(_)
        ));
    }
}

#[test]
fn ctrl_chord_passes_and_resets() {
    let mut d = dispatcher();
    let _ = replay(&mut d, "vie");
    let mut chord = KeyEvent::char('c');
    chord.ctrl = true;
    assert!(matches!(
        d.handle(chord, &Context::default()),
        Dispatch::Pass(_)
    ));
    assert!(!d.session.is_composing());
}

#[test]
fn win_chord_passes_and_resets() {
    // Win+R / Win+E / Win+arrows / ... belong to the shell - the LL hook
    // must let them through or every Win+key shortcut dies (and the engine
    // would inject the raw letter into the foreground app).
    let mut d = dispatcher();
    let _ = replay(&mut d, "vie");
    let mut chord = KeyEvent::char('r');
    chord.win = true;
    assert!(matches!(
        d.handle(chord, &Context::default()),
        Dispatch::Pass(_)
    ));
    assert!(!d.session.is_composing());
}

#[test]
fn injected_echoes_are_passed() {
    let mut d = dispatcher();
    let mut key = KeyEvent::char('a');
    key.injected = true;
    assert!(matches!(
        d.handle(key, &Context::default()),
        Dispatch::Pass(_)
    ));
    assert!(!d.session.is_composing());
}

/// Drive `text` through the dispatcher then send one Break key (Enter).
/// Returns the reconstructed screen; Enter is consumed by macro expansion
/// or appended as a newline when it passes through.
fn replay_enter(d: &mut Dispatcher, text: &str) -> String {
    let mut screen = String::new();
    for ch in text.chars() {
        match d.handle(KeyEvent::char(ch), &Context::default()) {
            Dispatch::Consume(plan) => apply(&mut screen, &plan),
            Dispatch::Pass(plan) => {
                apply(&mut screen, &plan);
                screen.push(ch);
            }
        }
    }
    let enter = KeyEvent {
        kind: KeyKind::Break,
        ..KeyEvent::char('\0')
    };
    match d.handle(enter, &Context::default()) {
        Dispatch::Consume(plan) => apply(&mut screen, &plan),
        Dispatch::Pass(plan) => {
            apply(&mut screen, &plan);
            screen.push('\n');
        }
    }
    screen
}

#[test]
fn macro_expands_on_space_and_swallows_it() {
    let mut d = dispatcher();
    d.macro_enabled = true;
    d.macros.add("sg", "Sài Gòn");
    // The space itself is consumed - the expansion replaces
    // abbreviation+terminator, like uvie-mac's applyMacroExpansion.
    assert_eq!(replay(&mut d, "sg "), "Sài Gòn");
}

#[test]
fn macro_expands_on_enter() {
    let mut d = dispatcher();
    d.macro_enabled = true;
    d.macros.add("hn", "Hà Nội");
    assert_eq!(replay_enter(&mut d, "hn"), "Hà Nội");
}

#[test]
fn macro_disabled_leaves_abbreviation_alone() {
    let mut d = dispatcher();
    d.macros.add("sg", "Sài Gòn");
    // macro_enabled defaults off - text must come through untouched.
    assert_eq!(replay(&mut d, "sg "), "sg ");
}

#[test]
fn macro_table_load_save_roundtrip() {
    let dir = std::env::temp_dir().join("uvie-macro-test");
    let path = dir.join("macros.json");
    let mut t = crate::macros::MacroTable::default();
    t.add("gd", "gửi đến");
    t.save(&path).unwrap();
    let back = crate::macros::MacroTable::load(&path);
    assert_eq!(back.lookup("gd"), Some("gửi đến"));
    // Missing file falls back to defaults without panicking.
    let missing = crate::macros::MacroTable::load(&dir.join("nope.json"));
    assert!(missing.lookup("gd").is_none());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn language_memory_roundtrip() {
    let mut mem = LanguageMemory::default();
    mem.remember("Code.EXE", LanguagePref::English);
    assert_eq!(mem.recall("code.exe"), Some(LanguagePref::English));
    assert_eq!(mem.recall("notepad.exe"), None);
}

#[test]
fn macro_lookup() {
    let mut t = crate::macros::MacroTable::default();
    t.add("sg", "Sài Gòn");
    t.add("sg", "Sài Gòn mới"); // replace, not duplicate
    assert_eq!(t.lookup("sg"), Some("Sài Gòn mới"));
    assert_eq!(t.entries.len(), 1);
    assert_eq!(t.lookup("hn"), None);
}
