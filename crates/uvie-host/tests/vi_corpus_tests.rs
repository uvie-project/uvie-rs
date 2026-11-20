//! Vietnamese mixed-prose regression corpus.
//!
//! Three files from the same source text: the expected output (`vi_mix.txt`)
//! and two Telex keystroke encodings of it (`vi_mix_telex.txt`, tone key
//! right after the vowel; `vi_mix_telex2.txt`, the same words typed another
//! way). The text mixes Vietnamese prose with English tech words ("merge",
//! "conflict", "test", "Jira"), which is where the English restore rules must
//! not fire: every word must come out exactly as the expected file.
//!
//! The harness mirrors `examples/zz_vi.rs`: a `Dispatcher` over the diff
//! engine with the English override on, applying every `InputAction` to a
//! simulated text buffer.

use uvie::InputMethod;
use uvie_host::dispatcher::{Context, Dispatch, Dispatcher, InputAction};
use uvie_host::engine_session::{EngineOptions, EngineSession};
use uvie_host::keys::KeyEvent;

const EXPECTED: &str = include_str!("data/vi_mix.txt");
const TELEX_1: &str = include_str!("data/vi_mix_telex.txt");
const TELEX_2: &str = include_str!("data/vi_mix_telex2.txt");

/// Type `text` through the dispatcher and return the on-screen result.
fn run(text: &str) -> String {
    let mut d = Dispatcher::new(EngineSession::with_options(
        InputMethod::Telex,
        EngineOptions {
            english_override: true,
            ..Default::default()
        },
    ));
    let mut screen: Vec<char> = Vec::new();
    let mut caret = 0usize;
    let ctx = Context {
        language: None,
        app_excluded: false,
        non_latin_layout: false,
    };
    for ch in text.chars() {
        let key = if ch == '\n' {
            KeyEvent::enter()
        } else {
            KeyEvent::char(ch)
        };
        let (plan, pass) = match d.handle(key, &ctx) {
            Dispatch::Consume(p) => (p, false),
            Dispatch::Pass(p) => (p, true),
        };
        for action in plan {
            match action {
                InputAction::Backspace(n) => {
                    for _ in 0..n {
                        if caret > 0 {
                            caret -= 1;
                            screen.remove(caret);
                        }
                    }
                }
                InputAction::ForwardDelete(n) => {
                    for _ in 0..n {
                        if caret < screen.len() {
                            screen.remove(caret);
                        }
                    }
                }
                InputAction::Text(t) => {
                    for c in t.chars() {
                        screen.insert(caret, c);
                        caret += 1;
                    }
                }
                #[allow(unreachable_patterns)]
                _ => {}
            }
        }
        if pass {
            screen.insert(caret, ch);
            caret += 1;
        }
    }
    screen.into_iter().collect()
}

/// Compare one keystroke corpus against the expected text; returns the wrong
/// words (`typed -> rendered (want expected)`).
fn wrong_words(keystrokes: &str) -> Vec<String> {
    let mut wrong = Vec::new();
    for (input, expected) in keystrokes.lines().zip(EXPECTED.lines()) {
        if input.trim().is_empty() {
            continue;
        }
        let out = run(&format!("{input} "));
        let out = out.trim_end_matches(' ');
        let want: Vec<&str> = expected.split(' ').collect();
        let got: Vec<&str> = out.split(' ').collect();
        let typed: Vec<&str> = input.split(' ').collect();
        if want.len() != got.len() {
            wrong.push(format!(
                "line length {} != {}: {expected:?} -> {out:?}",
                want.len(),
                got.len()
            ));
            continue;
        }
        for i in 0..want.len() {
            if want[i] != got[i] {
                wrong.push(format!(
                    "{} -> {} (want {})",
                    typed.get(i).copied().unwrap_or("?"),
                    got[i],
                    want[i]
                ));
            }
        }
    }
    wrong
}

#[test]
fn vietnamese_mixed_prose_tone_after_vowel() {
    let wrong = wrong_words(TELEX_1);
    assert!(wrong.is_empty(), "wrong words: {wrong:#?}");
}

#[test]
fn vietnamese_mixed_prose_second_encoding() {
    let wrong = wrong_words(TELEX_2);
    assert!(wrong.is_empty(), "wrong words: {wrong:#?}");
}
