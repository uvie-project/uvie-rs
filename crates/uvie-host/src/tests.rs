//! Headless behavior tests, ported from uvie-mac's `DispatcherTests` /
//! `HelperTests`: drive the real engine through the dispatcher and assert the
//! consume/pass decisions plus the injection plan.

use crate::dispatcher::{Context, Dispatch, Dispatcher, InputAction, InputLanguage};
use crate::engine_session::{EngineOptions, EngineSession};
use crate::keys::KeyEvent;
use crate::memory::{LanguageMemory, LanguagePref};
use crate::sentence::SentenceState;
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

fn type_text(d: &mut Dispatcher, text: &str) {
    for ch in text.chars() {
        d.handle(KeyEvent::char(ch), &Context::default());
    }
}

/// Apply a plan to a screen buffer at `caret`.
fn apply_at(screen: &mut Vec<char>, caret: &mut usize, plan: &[InputAction]) {
    for action in plan {
        match action {
            InputAction::Backspace(n) => {
                for _ in 0..*n {
                    if *caret > 0 {
                        *caret -= 1;
                        screen.remove(*caret);
                    }
                }
            }
            InputAction::ForwardDelete(n) => {
                for _ in 0..*n {
                    if *caret < screen.len() {
                        screen.remove(*caret);
                    }
                }
            }
            InputAction::Text(t) => {
                for ch in t.chars() {
                    screen.insert(*caret, ch);
                    *caret += 1;
                }
            }
        }
    }
}

/// Replay a caret-aware script like a real event source: `<` Backspace,
/// `{` Left, `}` Right, any other char a printable key. Returns the final
/// screen text with `|` marking the caret.
fn replay_script(d: &mut Dispatcher, script: &str) -> String {
    let mut screen: Vec<char> = Vec::new();
    let mut caret = 0usize;
    for ch in script.chars() {
        let key = match ch {
            '<' => KeyEvent::backspace(),
            '{' => KeyEvent::left(),
            '}' => KeyEvent::right(),
            c => KeyEvent::char(c),
        };
        match d.handle(key, &Context::default()) {
            Dispatch::Consume(plan) => apply_at(&mut screen, &mut caret, &plan),
            Dispatch::Pass(plan) => {
                apply_at(&mut screen, &mut caret, &plan);
                match ch {
                    '<' => {
                        if caret > 0 {
                            caret -= 1;
                            screen.remove(caret);
                        }
                    }
                    '{' => caret = caret.saturating_sub(1),
                    '}' => {
                        if caret < screen.len() {
                            caret += 1;
                        }
                    }
                    c => {
                        screen.insert(caret, c);
                        caret += 1;
                    }
                }
            }
        }
    }
    let mut out: String = screen.into_iter().collect();
    out.insert(
        out.char_indices()
            .nth(caret)
            .map(|x| x.0)
            .unwrap_or(out.len()),
        '|',
    );
    out
}

fn expect_consume(d: Dispatch) -> Vec<InputAction> {
    match d {
        Dispatch::Consume(plan) => plan,
        other => panic!("expected Consume, got {other:?}"),
    }
}

fn expect_pass(d: Dispatch) -> Vec<InputAction> {
    match d {
        Dispatch::Pass(plan) => plan,
        other => panic!("expected Pass, got {other:?}"),
    }
}

fn text(s: &str) -> InputAction {
    InputAction::Text(s.to_string())
}

/// Type `text` through the dispatcher, then press Backspace `n` times like a
/// real event source: every press must be PASSED THROUGH with an empty plan
/// (the app deletes the char natively), and the screen must lose exactly one
/// char per press. Returns the screen after each press.
fn backspace_walk(d: &mut Dispatcher, text: &str, n: usize) -> Vec<String> {
    let mut screen: Vec<char> = Vec::new();
    let mut caret = 0usize;
    for ch in text.chars() {
        match d.handle(KeyEvent::char(ch), &Context::default()) {
            Dispatch::Consume(plan) => apply_at(&mut screen, &mut caret, &plan),
            Dispatch::Pass(plan) => {
                apply_at(&mut screen, &mut caret, &plan);
                screen.insert(caret, ch);
                caret += 1;
            }
        }
    }
    let mut steps = Vec::new();
    for _ in 0..n {
        assert!(
            expect_pass(d.handle(KeyEvent::backspace(), &Context::default())).is_empty(),
            "a Backspace on committed text must never inject a synthetic edit"
        );
        if caret > 0 {
            caret -= 1;
            screen.remove(caret);
        }
        steps.push(screen.iter().collect::<String>());
    }
    steps
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
fn transform_key_backspaces_and_reposts() {
    // "vie" then a second 'e' → ê: one backspace + "ê".
    let mut d = dispatcher();
    type_text(&mut d, "vie");
    let plan = expect_consume(d.handle(KeyEvent::char('e'), &Context::default()));
    assert_eq!(plan, vec![InputAction::Backspace(1), text("ê")]);
}

#[test]
fn non_ascii_key_passes_through() {
    let mut d = dispatcher();
    let plan = expect_pass(d.handle(KeyEvent::char('ф'), &Context::default()));
    assert!(
        plan.is_empty(),
        "nothing must be injected for an unhandled key"
    );
}

#[test]
fn non_ascii_while_composing_commits_and_passes() {
    let mut d = dispatcher();
    type_text(&mut d, "cha");
    assert!(d.session.is_composing());
    let plan = expect_pass(d.handle(KeyEvent::char('é'), &Context::default()));
    assert!(
        !d.session.is_composing(),
        "the composing word must be committed"
    );
    assert_eq!(d.caret_back(), 0);
    assert!(
        plan.is_empty(),
        "no backspaces / no replacement text for the unhandled key: {plan:?}"
    );
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
    type_text(&mut d, "vie");
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
    // must let them through or every Win+key shortcut dies.
    let mut d = dispatcher();
    type_text(&mut d, "vie");
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

#[test]
fn backspace_walks_back_through_composition() {
    let mut d = dispatcher();
    let mut screen = String::new();
    for ch in "vie".chars() {
        match d.handle(KeyEvent::char(ch), &Context::default()) {
            Dispatch::Consume(plan) | Dispatch::Pass(plan) => apply(&mut screen, &plan),
        }
    }
    let plan = expect_consume(d.handle(KeyEvent::backspace(), &Context::default()));
    apply(&mut screen, &plan);
    assert_eq!(screen, "vi");
}

#[test]
fn backspace_when_not_composing_passes_through() {
    let mut d = dispatcher();
    assert!(expect_pass(d.handle(KeyEvent::backspace(), &Context::default())).is_empty());
}

#[test]
fn backspace_after_invisible_raw_key_deletes_visible_char() {
    // "aow" renders "ao": the trailing 'w' produced no glyph, so the first
    // backspace pops it with an empty diff. The drain must keep going.
    let mut d = dispatcher();
    type_text(&mut d, "aow");
    assert_eq!(d.session.current_output(), "ao");
    let plan = expect_consume(d.handle(KeyEvent::backspace(), &Context::default()));
    assert_eq!(plan, vec![InputAction::Backspace(1)]);
    assert_eq!(d.session.current_output(), "a");
}

#[test]
fn backspace_after_invisible_raw_key_uppercase_word() {
    // "ddf" renders "đ" because the trailing 'f' produced no glyph.
    let mut d = dispatcher();
    type_text(&mut d, "ddf");
    assert_eq!(d.session.current_output(), "đ");
    let plan = expect_consume(d.handle(KeyEvent::backspace(), &Context::default()));
    assert_eq!(plan, vec![InputAction::Backspace(1), text("d")]);
}

#[test]
fn space_commits_and_passes_through() {
    let mut d = dispatcher();
    type_text(&mut d, "vieejt");
    assert!(d.session.is_composing());
    expect_pass(d.handle(KeyEvent::char(' '), &Context::default()));
    assert!(!d.session.is_composing());
}

#[test]
fn space_macro_expansion_consumes_and_injects() {
    let mut d = dispatcher();
    d.macro_enabled = true;
    d.macros.add("việt", "X");
    type_text(&mut d, "vieejt");
    let plan = expect_consume(d.handle(KeyEvent::char(' '), &Context::default()));
    assert_eq!(plan.last(), Some(&text("X")));
}

#[test]
fn enter_passes_through_and_commits() {
    let mut d = dispatcher();
    type_text(&mut d, "vie");
    expect_pass(d.handle(KeyEvent::enter(), &Context::default()));
    assert!(!d.session.is_composing());
    assert!(d.sentence.is_at_start());
}

#[test]
fn escape_resets_without_commit() {
    let mut d = dispatcher();
    type_text(&mut d, "vie");
    expect_pass(d.handle(KeyEvent::escape(), &Context::default()));
    assert!(!d.session.is_composing());
}

#[test]
fn escape_restores_english_raw_while_composing() {
    // "vieet" renders "viêt"; Escape erases the render and posts the raw
    // keystrokes back, consuming the key.
    let mut d = dispatcher();
    type_text(&mut d, "vieet");
    let plan = expect_consume(d.handle(KeyEvent::escape(), &Context::default()));
    assert_eq!(plan, vec![InputAction::Backspace(4), text("vieet")]);
    assert!(!d.session.is_composing());
}

#[test]
fn escape_restores_whole_word_after_syllable_split() {
    // "bister" renders "bítẻ" - the split commits "bí" and only "ter" stays
    // composing. Escape must restore the WHOLE word.
    let mut d = dispatcher();
    type_text(&mut d, "bister");
    let plan = expect_consume(d.handle(KeyEvent::escape(), &Context::default()));
    assert_eq!(plan, vec![InputAction::Backspace(4), text("bister")]);
    assert!(!d.session.is_composing());
}

#[test]
fn escape_passes_through_when_render_matches_raw() {
    // "abc" renders literally - nothing to restore.
    let mut d = dispatcher();
    type_text(&mut d, "abc");
    assert!(expect_pass(d.handle(KeyEvent::escape(), &Context::default())).is_empty());
}

#[test]
fn escape_passes_through_when_idle() {
    let mut d = dispatcher();
    assert!(expect_pass(d.handle(KeyEvent::escape(), &Context::default())).is_empty());
}

#[test]
fn arrow_key_commits_and_passes_through() {
    let mut d = dispatcher();
    type_text(&mut d, "vie");
    expect_pass(d.handle(KeyEvent::left(), &Context::default()));
    assert!(!d.session.is_composing());
    assert_eq!(d.caret_back(), 1);
}

#[test]
fn edit_committed_word_tone_key_replaces_in_place() {
    // "don" + space commits "don"; arrow-left onto the word end; 's'
    // re-renders it as "dón": 2 backspaces eat "on", then "ón" is typed.
    let mut d = dispatcher();
    type_text(&mut d, "don");
    expect_pass(d.handle(KeyEvent::char(' '), &Context::default()));
    expect_pass(d.handle(KeyEvent::left(), &Context::default()));
    let plan = expect_consume(d.handle(KeyEvent::char('s'), &Context::default()));
    assert_eq!(plan, vec![InputAction::Backspace(2), text("ón")]);
}

#[test]
fn edit_committed_word_backspace_instead_of_arrow_also_arms() {
    // Commit "don", then delete the space with backspace - the caret lands on
    // the word end and the edit arms the same way.
    let mut d = dispatcher();
    type_text(&mut d, "don");
    expect_pass(d.handle(KeyEvent::char(' '), &Context::default()));
    expect_pass(d.handle(KeyEvent::backspace(), &Context::default()));
    assert_eq!(d.caret_back(), 0);
    let plan = expect_consume(d.handle(KeyEvent::char('s'), &Context::default()));
    assert_eq!(plan, vec![InputAction::Backspace(2), text("ón")]);
}

#[test]
fn edit_committed_word_mid_word_caret_edits() {
    let mut d = dispatcher();
    type_text(&mut d, "don");
    expect_pass(d.handle(KeyEvent::char(' '), &Context::default()));
    // Two arrow-lefts put the caret mid-word ("do|n", caret_back = 1).
    expect_pass(d.handle(KeyEvent::left(), &Context::default()));
    expect_pass(d.handle(KeyEvent::left(), &Context::default()));
    assert_eq!(d.caret_back(), 1);
    // Mid-word edit: the "n" tail is forward-deleted, "do" is backspaced,
    // and the re-rendered "dón" is posted.
    let plan = expect_consume(d.handle(KeyEvent::char('s'), &Context::default()));
    assert_eq!(
        plan,
        vec![
            InputAction::ForwardDelete(1),
            InputAction::Backspace(2),
            text("dón")
        ]
    );
}

#[test]
fn edit_committed_word_arrow_while_composing_reanchors() {
    // "ab " committed, "don" still composing; one left arrow lands the caret
    // at "do|n" and 's' must mid-word-edit "don" -> "dón".
    let mut d = dispatcher();
    type_text(&mut d, "ab ");
    type_text(&mut d, "don");
    assert!(d.session.is_composing());
    expect_pass(d.handle(KeyEvent::left(), &Context::default()));
    assert_eq!(d.caret_back(), 1);
    let plan = expect_consume(d.handle(KeyEvent::char('s'), &Context::default()));
    assert_eq!(
        plan,
        vec![
            InputAction::ForwardDelete(1),
            InputAction::Backspace(2),
            text("dón")
        ]
    );
}

#[test]
fn edit_committed_word_transformed_word_arrow_reanchors() {
    // Same re-anchor, but the composing word contains transform keystrokes
    // ("vieejt" -> "việt") that drifted the naive -1-per-key tracking.
    let mut d = dispatcher();
    expect_pass(d.handle(KeyEvent::char(' '), &Context::default()));
    type_text(&mut d, "vieejt");
    expect_pass(d.handle(KeyEvent::left(), &Context::default()));
    assert_eq!(d.caret_back(), 1);
    let plan = expect_consume(d.handle(KeyEvent::char('s'), &Context::default()));
    assert_eq!(
        plan,
        vec![
            InputAction::ForwardDelete(1),
            InputAction::Backspace(3),
            text("viết")
        ]
    );
}

#[test]
fn backspace_while_composing_updates_anchor_offset() {
    let mut d = dispatcher();
    type_text(&mut d, "ab ");
    type_text(&mut d, "don");
    assert_eq!(d.caret_back(), -4); // space + 3 rendered chars
    expect_consume(d.handle(KeyEvent::backspace(), &Context::default()));
    assert_eq!(d.caret_back(), -3);
}

#[test]
fn forward_delete_deleting_space_arms_edit() {
    let mut d = dispatcher();
    type_text(&mut d, "don");
    expect_pass(d.handle(KeyEvent::char(' '), &Context::default()));
    // Step back onto the word end, then Forward Delete removes the space.
    expect_pass(d.handle(KeyEvent::left(), &Context::default()));
    expect_pass(d.handle(KeyEvent::forward_delete(), &Context::default()));
    assert_eq!(d.caret_back(), 0);
    let plan = expect_consume(d.handle(KeyEvent::char('s'), &Context::default()));
    assert_eq!(plan, vec![InputAction::Backspace(2), text("ón")]);
}

#[test]
fn space_inside_committed_text_resets_history() {
    let mut d = dispatcher();
    type_text(&mut d, "don");
    expect_pass(d.handle(KeyEvent::char(' '), &Context::default()));
    expect_pass(d.handle(KeyEvent::left(), &Context::default()));
    expect_pass(d.handle(KeyEvent::left(), &Context::default()));
    assert_eq!(d.caret_back(), 1);
    expect_pass(d.handle(KeyEvent::char(' '), &Context::default()));
    assert_eq!(d.caret_back(), 0);
    assert!(!d.session.is_composing());
}

#[test]
fn tab_resets_and_passes_through() {
    let mut d = dispatcher();
    type_text(&mut d, "vie");
    expect_pass(d.handle(KeyEvent::tab(), &Context::default()));
    assert!(!d.session.is_composing());
}

#[test]
fn navigation_resets_and_passes_through() {
    let mut d = dispatcher();
    type_text(&mut d, "vie");
    expect_pass(d.handle(KeyEvent::navigation(), &Context::default()));
    assert!(!d.session.is_composing());
}

#[test]
fn shift_arrow_resets_instead_of_arming_edit() {
    let mut d = dispatcher();
    type_text(&mut d, "don");
    expect_pass(d.handle(KeyEvent::char(' '), &Context::default()));
    expect_pass(d.handle(KeyEvent::left(), &Context::default()));
    assert_eq!(d.caret_back(), 0);
    let mut sel = KeyEvent::left();
    sel.shift = true;
    expect_pass(d.handle(sel, &Context::default()));
    assert_eq!(d.caret_back(), 0);
    // History is gone: the tone key is inserted literally.
    let plan = expect_consume(d.handle(KeyEvent::char('s'), &Context::default()));
    assert_eq!(plan, vec![text("s")]);
}

#[test]
fn shift_forward_delete_resets() {
    let mut d = dispatcher();
    type_text(&mut d, "don");
    expect_pass(d.handle(KeyEvent::char(' '), &Context::default()));
    let mut cut = KeyEvent::forward_delete();
    cut.shift = true;
    expect_pass(d.handle(cut, &Context::default()));
    assert_eq!(d.caret_back(), 0);
    let plan = expect_consume(d.handle(KeyEvent::char('s'), &Context::default()));
    assert_eq!(plan, vec![text("s")]);
}

#[test]
fn macro_expands_on_space_and_swallows_it() {
    let mut d = dispatcher();
    d.macro_enabled = true;
    d.macros.add("sg", "Sài Gòn");
    // The space itself is consumed - the expansion replaces
    // abbreviation+terminator.
    assert_eq!(replay(&mut d, "sg "), "Sài Gòn");
}

#[test]
fn macro_expands_on_enter() {
    let mut d = dispatcher();
    d.macro_enabled = true;
    d.macros.add("hn", "Hà Nội");
    type_text(&mut d, "hn");
    let plan = expect_consume(d.handle(KeyEvent::enter(), &Context::default()));
    let mut screen = String::new();
    apply(&mut screen, &plan);
    assert_eq!(screen, "Hà Nội");
}

#[test]
fn macro_disabled_leaves_abbreviation_alone() {
    let mut d = dispatcher();
    d.macros.add("sg", "Sài Gòn");
    assert_eq!(replay(&mut d, "sg "), "sg ");
}

// MARK: - Auto-capitalize / sentence state

#[test]
fn auto_capitalize_uppercases_at_sentence_start() {
    let mut d = dispatcher();
    d.auto_capitalize = true;
    let plan = expect_consume(d.handle(KeyEvent::char('a'), &Context::default()));
    assert_eq!(plan, vec![text("A")]);
    assert!(!d.sentence.is_at_start(), "first letter consumed the state");
    // Subsequent letters are untouched.
    let plan = expect_consume(d.handle(KeyEvent::char('b'), &Context::default()));
    assert_eq!(plan, vec![text("b")]);
}

#[test]
fn auto_capitalize_disabled_is_identity() {
    let mut d = dispatcher();
    let plan = expect_consume(d.handle(KeyEvent::char('a'), &Context::default()));
    assert_eq!(plan, vec![text("a")]);
}

#[test]
fn auto_capitalize_non_letters_do_not_consume_sentence_start() {
    let mut d = dispatcher();
    d.auto_capitalize = true;
    expect_pass(d.handle(KeyEvent::char(' '), &Context::default()));
    assert!(d.sentence.is_at_start(), "a space must not end the window");
    let plan = expect_consume(d.handle(KeyEvent::char('a'), &Context::default()));
    assert_eq!(plan, vec![text("A")]);
}

#[test]
fn sentence_start_state_transitions() {
    let mut s = SentenceState::new();
    s.set_at_start(false);
    s.update_after('.');
    assert!(s.is_at_start());
    s.set_at_start(false);
    s.update_after('!');
    assert!(s.is_at_start());
    s.set_at_start(false);
    s.update_after('?');
    assert!(s.is_at_start());
    s.set_at_start(false);
    s.update_after('a');
    assert!(!s.is_at_start());
    s.set_at_start(true);
    s.update_after('1');
    assert!(!s.is_at_start());
    s.set_at_start(false);
    s.update_after(',');
    assert!(
        !s.is_at_start(),
        "other punctuation leaves the state unchanged"
    );
}

#[test]
fn enter_starts_new_sentence_tab_does_not() {
    let mut s = SentenceState::new();
    s.set_at_start(false);
    s.on_enter();
    assert!(s.is_at_start());

    let mut d = dispatcher();
    d.sentence.set_at_start(false);
    expect_pass(d.handle(KeyEvent::tab(), &Context::default()));
    assert!(!d.sentence.is_at_start());
}

#[test]
fn pointer_down_saves_and_resets() {
    let mut d = dispatcher();
    d.sentence.set_at_start(false);
    d.on_pointer_down();
    assert_eq!(d.sentence.saved(), Some(false));
    assert!(d.sentence.is_at_start());
}

#[test]
fn app_switch_restores_saved_sentence_start() {
    let mut d = dispatcher();
    d.sentence.set_at_start(false);
    d.on_pointer_down();
    assert_eq!(d.sentence.saved(), Some(false));
    assert!(d.sentence.is_at_start());
    d.on_app_switch("com.test.B");
    assert!(!d.sentence.is_at_start(), "restore the pre-click state");
    assert_eq!(d.sentence.saved(), None);
}

#[test]
fn key_down_clears_saved_sentence_start() {
    let mut d = dispatcher();
    d.sentence.set_at_start(false);
    d.on_pointer_down();
    assert_eq!(d.sentence.saved(), Some(false));
    // No app switch follows: the next key press discards the save.
    expect_pass(d.handle(KeyEvent::char(' '), &Context::default()));
    assert_eq!(d.sentence.saved(), None);
    assert!(d.sentence.is_at_start());
}

#[test]
fn app_switch_per_app_memory_round_trip() {
    let mut d = dispatcher();
    d.on_app_switch("com.test.A");
    d.sentence.set_at_start(false); // A is mid-sentence

    // Switch to B (never visited): A's state is filed, B keeps current.
    d.on_app_switch("com.test.B");
    assert!(
        !d.sentence.is_at_start(),
        "first visit to B keeps the state"
    );

    // Type a sentence ending in "." in B -> state becomes true.
    d.sentence.update_after('o');
    d.sentence.update_after('k');
    d.sentence.update_after('.');
    assert!(d.sentence.is_at_start());

    // Back to A: A's own mid-sentence state returns.
    d.on_app_switch("com.test.A");
    assert!(!d.sentence.is_at_start(), "A restores its own state");

    // Back to B: B's post-delimiter state returns.
    d.on_app_switch("com.test.B");
    assert!(d.sentence.is_at_start(), "B ended with a delimiter");
}

#[test]
fn click_switch_round_trip_mid_sentence_not_recapitalized() {
    let mut d = dispatcher();
    d.auto_capitalize = true;
    d.on_app_switch("com.test.A");
    d.sentence.set_at_start(false);

    // Click B's window (pointer down) -> app switch to B.
    d.on_pointer_down();
    d.on_app_switch("com.test.B");
    // Click back into A's window -> app switch to A.
    d.on_pointer_down();
    d.on_app_switch("com.test.A");

    let plan = expect_consume(d.handle(KeyEvent::char('a'), &Context::default()));
    assert_eq!(
        plan,
        vec![text("a")],
        "returning mid-sentence must not capitalize"
    );
}

// MARK: - English mode sentence state

#[test]
fn english_mode_updates_sentence_start_state() {
    let mut d = dispatcher();
    d.auto_capitalize = true;
    d.set_language(InputLanguage::English);

    // First letter is capitalized (and consumed); the rest pass through.
    let plan = expect_consume(d.handle(KeyEvent::char('h'), &Context::default()));
    assert_eq!(plan, vec![text("H")]);
    for ch in "ello".chars() {
        expect_pass(d.handle(KeyEvent::char(ch), &Context::default()));
    }
    assert!(!d.sentence.is_at_start());

    d.set_language(InputLanguage::Vietnamese);
    let plan = expect_consume(d.handle(KeyEvent::char('a'), &Context::default()));
    assert_eq!(plan, vec![text("a")], "mid-word must not be capitalized");
}

#[test]
fn english_mode_backspace_passes_through() {
    let mut d = dispatcher();
    d.set_language(InputLanguage::English);
    assert!(expect_pass(d.handle(KeyEvent::backspace(), &Context::default())).is_empty());
}

#[test]
fn english_mode_enter_starts_new_sentence() {
    let mut d = dispatcher();
    d.set_language(InputLanguage::English);
    d.sentence.set_at_start(false);
    expect_pass(d.handle(KeyEvent::enter(), &Context::default()));
    assert!(d.sentence.is_at_start());
}

// MARK: - Diff-state ownership / reset regressions

#[test]
fn is_composing_tracks_dictionary_override_state() {
    // The English-dictionary override commits the word into `diff_committed`
    // and clears the core raw buffer; the host must still consider the word
    // engine-owned or the next key can edit/pass through against stale state.
    let mut d = dispatcher();
    type_text(&mut d, "good");
    assert!(
        d.session.is_composing(),
        "a dictionary word is still engine-owned text"
    );
}

#[test]
fn set_input_method_same_value_preserves_composition() {
    let mut d = dispatcher();
    type_text(&mut d, "vie");
    assert!(d.session.is_composing());
    // Settings hot-reload re-applies the same method: must be a no-op.
    d.session.set_input_method(InputMethod::Telex);
    assert!(d.session.is_composing());
    // A real change still drops the state.
    d.session.set_input_method(InputMethod::Vni);
    assert!(!d.session.is_composing());
}

#[test]
fn pointer_down_disarms_post_commit_edit() {
    let mut d = dispatcher();
    type_text(&mut d, "don");
    expect_pass(d.handle(KeyEvent::char(' '), &Context::default()));
    expect_pass(d.handle(KeyEvent::left(), &Context::default()));
    assert_eq!(d.caret_back(), 0);
    d.on_pointer_down();
    let plan = expect_consume(d.handle(KeyEvent::char('s'), &Context::default()));
    assert_eq!(
        plan,
        vec![text("s")],
        "the click must drop the edit history"
    );
}

#[test]
fn non_latin_layout_passes_and_resets() {
    let mut d = dispatcher();
    type_text(&mut d, "vie");
    let ctx = Context {
        non_latin_layout: true,
        ..Default::default()
    };
    assert!(expect_pass(d.handle(KeyEvent::char('a'), &ctx)).is_empty());
    assert!(!d.session.is_composing());
}

#[test]
fn language_toggle_resets_composition() {
    let mut d = dispatcher();
    type_text(&mut d, "vie");
    d.set_language(InputLanguage::English);
    assert!(!d.session.is_composing());

    d.set_language(InputLanguage::Vietnamese);
    type_text(&mut d, "vie");
    assert!(d.session.is_composing());
    d.on_language_changed();
    assert!(!d.session.is_composing());
}

#[test]
fn punctuation_commits_and_starts_sentence() {
    let mut d = dispatcher();
    d.auto_capitalize = true;
    type_text(&mut d, "hello");
    assert!(d.session.is_composing());
    let plan = expect_pass(d.handle(KeyEvent::char('.'), &Context::default()));
    assert!(plan.is_empty());
    assert!(!d.session.is_composing());
    assert!(d.sentence.is_at_start(), "'.' starts a new sentence");

    let plan = expect_consume(d.handle(KeyEvent::char('a'), &Context::default()));
    assert_eq!(plan, vec![text("A")]);
}

#[test]
fn edit_committed_word_mid_sentence_second_word() {
    // "ab cd eg " - arrow back onto "cd" (the middle word) and edit it.
    // Boundary for cd's end = len("eg") + 1 = 3 behind the anchor; the
    // post-commit caret starts 1 right of it, so 4 arrow-lefts.
    let mut d = dispatcher();
    for word in ["ab", "cd", "eg"] {
        type_text(&mut d, word);
        expect_pass(d.handle(KeyEvent::char(' '), &Context::default()));
    }
    for _ in 0..("eg".len() + 1 + 1) {
        expect_pass(d.handle(KeyEvent::left(), &Context::default()));
    }
    assert_eq!(d.caret_back(), 3);
    // Typing at cd's end re-enters "cd" -> "cds": no backspaces needed
    // (common prefix), just the suffix.
    let plan = expect_consume(d.handle(KeyEvent::char('s'), &Context::default()));
    assert_eq!(plan, vec![text("s")]);
}

#[test]
fn edit_declines_at_word_start_and_resets() {
    let mut d = dispatcher();
    type_text(&mut d, "don");
    expect_pass(d.handle(KeyEvent::char(' '), &Context::default()));
    for _ in 0..4 {
        expect_pass(d.handle(KeyEvent::left(), &Context::default()));
    }
    assert_eq!(d.caret_back(), 3);
    // The word's first raw key cannot be edited: the key is inserted
    // literally and the stale anchor is dropped.
    let plan = expect_consume(d.handle(KeyEvent::char('s'), &Context::default()));
    assert_eq!(plan, vec![text("s")]);
    assert_eq!(d.caret_back(), 0);
}

#[test]
fn edit_disabled_still_drops_stale_anchor() {
    let mut d = dispatcher();
    type_text(&mut d, "don");
    expect_pass(d.handle(KeyEvent::char(' '), &Context::default()));
    expect_pass(d.handle(KeyEvent::left(), &Context::default()));
    expect_pass(d.handle(KeyEvent::left(), &Context::default()));
    assert_eq!(d.caret_back(), 1);
    d.edit_committed = false;
    let plan = expect_consume(d.handle(KeyEvent::char('s'), &Context::default()));
    assert_eq!(plan, vec![text("s")]);
    assert_eq!(d.caret_back(), 0);
}

// MARK: - Caret-aware post-commit editing (LabanKey-style)

#[test]
fn edit_after_punctuation_and_space() {
    // Punctuation commits must keep the committed-word history: the caret
    // sits at the end of the word with ". " to its right.
    assert_eq!(
        replay_script(&mut dispatcher(), "ddi hoc. {{j"),
        "đi học|. "
    );
    assert_eq!(replay_script(&mut dispatcher(), "vieet. {{j"), "việt|. ");
}

#[test]
fn edit_after_double_space() {
    // A second Space only grows the trailing gap; the caret is still measured
    // from the word's end.
    assert_eq!(replay_script(&mut dispatcher(), "tieng  {{j"), "tiệng|  ");
}

#[test]
fn edit_older_word_keeps_newer_words_editable() {
    // Edit "viêt" (the older word), then move back to "nam"'s end: 'j' must
    // still re-render "nam" in place.
    assert_eq!(
        replay_script(&mut dispatcher(), "vieet nam {{{{{j}}}}}{j"),
        "việt nạm| "
    );
}

#[test]
fn backspace_keeps_a_committed_word_editable() {
    // The first Backspace deletes the separator, the second is passed through
    // so the app deletes the word's last visible char natively ("viêt" ->
    // "viê"); the word stays editable, so 't' re-renders it.
    assert_eq!(replay_script(&mut dispatcher(), "vieet <<tj"), "việt|");
    // A word being edited in place is an intermediate state: only the
    // dictionary applies, the restore-raw rule judges finished words.
    assert_eq!(replay_script(&mut dispatcher(), "vieet <<j"), "việ|");
    // ... and retyping the deleted char restores the word.
    assert_eq!(replay_script(&mut dispatcher(), "hoc <<c"), "hoc|");
}

#[test]
fn backspace_walks_committed_text_one_char_per_press() {
    // A held Backspace over committed words is a plain native delete: every
    // press passes through with no injected edit, so nothing races the key
    // repeat and no Vietnamese text is ever re-rendered from English input.
    let steps = backspace_walk(&mut dispatcher(), "tieengs vieetj ", 15);
    let mut want = String::from("tiếng việt ");
    for step in &steps {
        want.pop();
        assert_eq!(step, &want);
    }
    assert!(steps.last().is_some_and(|s| s.is_empty()));
}

#[test]
fn backspace_on_english_words_stays_english() {
    // "hello world " -> "hello wo": each press removes one char, and deleting
    // chars of an English word never re-renders it as Telex (the old
    // keystroke-undo produced "hello ư").
    let steps = backspace_walk(&mut dispatcher(), "hello world ", 4);
    assert_eq!(
        steps.iter().map(String::as_str).collect::<Vec<_>>(),
        ["hello world", "hello worl", "hello wor", "hello wo"]
    );
}

#[test]
fn backspace_then_edit_after_a_native_delete() {
    // The shortened word stays editable and a tone key renders it exactly like
    // typing the shortened word fresh: "viêt" -> "viê" -> 'j' -> "việ".
    assert_eq!(replay_script(&mut dispatcher(), "vieet <<j"), "việ|");
    // When the canonical raw already ends with its tone key, typing that key
    // again is a DOUBLED tone key: the engine cancels it into the literal raw
    // ("Vieejj"), exactly like typing that raw fresh.
    assert_eq!(replay_script(&mut dispatcher(), "vieetj <<j"), "vieej|");
    // Deleted down to a bare consonant: "v" + 'j' is the literal "vj".
    assert_eq!(replay_script(&mut dispatcher(), "vieet <<<<j"), "vj|");
    // "được" -> "đượ" -> 'j' is again a doubled tone key ("dduwowjj").
    assert_eq!(replay_script(&mut dispatcher(), "dduwowcj <<j"), "đuwoj|");
    // A consonant edit appends normally: "tiế" + 'n' -> "tiến".
    assert_eq!(replay_script(&mut dispatcher(), "tieengs <<<n"), "tiến|");
}

#[test]
fn backspace_deletes_native_chars_across_words() {
    // Erasing a whole word drops its ring entry and merges its separators into
    // the older word, which stays editable: "Việt Nam " -> "Việ" -> 's'
    // replaces the nặng with a sắc.
    assert_eq!(
        replay_script(&mut dispatcher(), "Vieetj Nam <<<<<<s"),
        "Viế|"
    );
    // Deleting one char of an OLDER word keeps the caret's offset: "ab"'s end
    // stays editable after its native delete.
    assert_eq!(replay_script(&mut dispatcher(), "ab cd {{{{<s"), "á| cd ");
}

#[test]
fn edit_word_end_after_caret_moves() {
    assert_eq!(replay_script(&mut dispatcher(), "vieet {j"), "việt| ");
    assert_eq!(replay_script(&mut dispatcher(), "vieet{j"), "việt|");
    assert_eq!(replay_script(&mut dispatcher(), "vieet <j"), "việt|");
    // Two edits in a row: a second 'j' would cancel the tone and leave a
    // literal 'j', so the second key is 's'.
    assert_eq!(replay_script(&mut dispatcher(), "vieet {js"), "viết| ");
}

#[test]
fn edit_older_words_in_a_long_sentence() {
    assert_eq!(
        replay_script(&mut dispatcher(), "chao ban {{{{{f"),
        "chào| ban "
    );
    assert_eq!(
        replay_script(&mut dispatcher(), "ban, toi {{{{{{f"),
        "bàn|, toi "
    );
}

#[test]
fn edit_three_words_in_arbitrary_order() {
    // "mot hai ba bon " — tone the middle word, then the oldest, then the
    // newest; every edit leaves the rest of the sentence alone.
    assert_eq!(
        replay_script(
            &mut dispatcher(),
            "mot hai ba bon {{{{{j{{{{{{{s}}}}}}}}}}}f"
        ),
        "mót hai bạ bòn| "
    );
}

#[test]
fn backspace_deleting_the_separator_tracks_the_gap() {
    let mut d = dispatcher();
    type_text(&mut d, "don ");
    assert_eq!(d.caret_back(), -1);
    // Deleting the space leaves the caret at the word's end.
    expect_pass(d.handle(KeyEvent::backspace(), &Context::default()));
    assert_eq!(d.caret_back(), 0);
    // The word is still editable there.
    let plan = expect_consume(d.handle(KeyEvent::char('s'), &Context::default()));
    assert_eq!(plan, vec![InputAction::Backspace(2), text("ón")]);
}

// MARK: - Macros / memory persistence

#[test]
fn macro_table_load_save_roundtrip() {
    let dir = std::env::temp_dir().join("uvie-macro-test");
    let path = dir.join("macros.json");
    let mut t = crate::macros::MacroTable::default();
    t.add("gd", "gửi đến");
    t.save(&path).unwrap();
    let back = crate::macros::MacroTable::load(&path);
    assert_eq!(back.lookup("gd"), Some("gửi đến"));
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

// MARK: - Cross-word rewrite safety

/// Feed one key through the dispatcher, apply its plan to `screen` and push
/// the passed-through char for Pass dispatches. Returns the plan.
fn feed_key(
    d: &mut Dispatcher,
    screen: &mut String,
    key: KeyEvent,
    pass: Option<char>,
) -> Vec<InputAction> {
    match d.handle(key, &Context::default()) {
        Dispatch::Consume(plan) => {
            apply(screen, &plan);
            plan
        }
        Dispatch::Pass(plan) => {
            apply(screen, &plan);
            if let Some(c) = pass {
                screen.push(c);
            }
            plan
        }
    }
}

fn feed_str(d: &mut Dispatcher, screen: &mut String, text: &str) {
    for ch in text.chars() {
        feed_key(d, screen, KeyEvent::char(ch), Some(ch));
    }
}

#[test]
fn newline_never_rewrites_across_the_line_break() {
    let mut d = dispatcher();
    let mut screen = String::new();
    feed_str(&mut d, &mut screen, "this");
    feed_key(&mut d, &mut screen, KeyEvent::enter(), Some('\n'));
    feed_str(&mut d, &mut screen, "test");
    let plan = feed_key(&mut d, &mut screen, KeyEvent::char(' '), Some(' '));
    for action in &plan {
        match action {
            // "test" renders at most 4 screen chars; a larger erase reaches
            // past the newline into the previous line.
            InputAction::Backspace(n) => assert!(*n <= 4, "plan {plan:?}"),
            InputAction::Text(t) => assert!(!t.contains('\n'), "plan {plan:?}"),
            _ => {}
        }
    }
    assert_eq!(screen.lines().next_back().unwrap(), "test ");
}

#[test]
fn apostrophe_deleted_before_suffix_leaves_vietnamese_alone() {
    let mut d = dispatcher();
    // "vieetj'" then Backspace drops the apostrophe: the gap is mixed and its
    // remaining char is unknowable, so 's' must not read as a contraction.
    assert_eq!(replay_script(&mut d, "vieetj '<s "), "việt s |");
}

#[test]
fn overflowed_word_declines_contraction_restore() {
    let mut d = dispatcher();
    // A word longer than the 24-key raw buffer only knows its tail; the
    // contraction rewrite must not rebuild it from an incomplete raw.
    assert_eq!(
        replay(&mut d, "getCurrentApplicationSettings's "),
        "getCurrentApplicationSettings's "
    );
}

#[test]
fn overflowed_word_alone_stays_intact() {
    let mut d = dispatcher();
    assert_eq!(
        replay(&mut d, "getCurrentApplicationSettings "),
        "getCurrentApplicationSettings "
    );
}

#[test]
fn edit_committed_off_blocks_cross_word_rewrites() {
    let mut d = dispatcher();
    d.edit_committed = false;
    let mut screen = String::new();
    feed_str(&mut d, &mut screen, "this is a test");
    // The final Space must not erase past the current word ("test" renders
    // at most 4 chars); committed text cannot be rewritten on this host.
    let plan = feed_key(&mut d, &mut screen, KeyEvent::char(' '), Some(' '));
    for action in &plan {
        if let InputAction::Backspace(n) = action {
            assert!(*n <= 4, "plan {plan:?}");
        }
    }
    // Single-word restores still work ("test"), but the ambiguous words the
    // retro fix would have restored stay Vietnamese on a no-rewrite host.
    assert_eq!(screen, "thí í a test ");
}

#[test]
fn edit_committed_on_keeps_retro_restore_working() {
    let mut d = dispatcher();
    assert_eq!(replay(&mut d, "this is a test "), "this is a test ");
}

#[test]
fn separator_delete_keeps_the_comma() {
    let mut d = dispatcher();
    // "this," + Space + Backspace leaves "this,": deleting the space at the
    // end of the gap is an exact undo, so the ',' separator is still known
    // exactly and the retro fix restores "this" across it - the same text
    // typing "this,is a test " directly produces.
    assert_eq!(
        replay_script(&mut d, "this, <is a test "),
        "this,is a test |"
    );
}

#[test]
fn separator_delete_undoes_code_token_dot() {
    // A '.' deleted at the end of the gap must also undo the code-token
    // state it created; on a no-rewrite host the next word renders again.
    for script in ["chaof.<dduwowcj ", "chaof. <<dduwowcj "] {
        let mut d = dispatcher();
        d.edit_committed = false;
        assert_eq!(replay_script(&mut d, script), "chàođược |", "{script:?}");
    }
    // With editing of committed text on, typing right after the deleted
    // dot re-enters "chào" and extends the token instead (LabanKey edit).
}

#[test]
fn separator_delete_inside_gap_stays_conservative() {
    // "this, " with the caret moved inside the gap: Backspace deletes the
    // comma (not the last separator), so the conservative path runs and the
    // surviving separator is unknown - the space stays, no retro restore.
    let mut d = dispatcher();
    assert_eq!(
        replay_script(&mut d, "this, {<}is a test "),
        "thí is a test |"
    );
}

#[test]
fn sentence_end_separator_delete_restores_context() {
    // Deleting the '!' at the end of the gap undoes the sentence end it
    // recorded: "chào" is still part of the current sentence's text.
    let mut d = dispatcher();
    replay_script(&mut d, "chaof!< dduwowcj ");
    let text = d.session.sentence_text();
    assert!(text.contains("chào"), "{text:?}");
    assert!(text.contains("được"), "{text:?}");

    // Control: with the '!' kept, the sentence ended at "chào".
    let mut d = dispatcher();
    replay_script(&mut d, "chaof! dduwowcj ");
    let text = d.session.sentence_text();
    assert!(text.contains("được"), "{text:?}");
    assert!(!text.contains("chào"), "{text:?}");
}

#[test]
fn long_gap_delete_past_the_undo_stack_stays_sane() {
    // Six separators: the undo stack holds four and the rest falls back to
    // the conservative removal - no panic, the word still renders. (On a
    // host that edits committed text the typed letters would instead
    // re-enter "chào" as one token; the fresh-word render is the
    // edit_committed=false contract.)
    let mut d = dispatcher();
    d.edit_committed = false;
    assert_eq!(
        replay_script(&mut d, "chaof      <<<<<<dduwowcj "),
        "chàođược |"
    );
}
#[test]
fn double_space_backspace_keeps_exact_space_context() {
    let mut d = dispatcher();
    // Two identical separators survive a delete as one exact space: the
    // English context rules still work across it.
    assert_eq!(
        replay_script(&mut d, "this  <is a test "),
        "this is a test |"
    );
}

#[test]
fn break_key_separator_blocks_cross_word_rewrite() {
    let mut d = dispatcher();
    let mut screen = String::new();
    feed_str(&mut d, &mut screen, "this");
    feed_key(&mut d, &mut screen, KeyEvent::break_key(), None);
    feed_str(&mut d, &mut screen, "test");
    let plan = feed_key(&mut d, &mut screen, KeyEvent::char(' '), Some(' '));
    for action in &plan {
        match action {
            InputAction::Backspace(n) => assert!(*n <= 4, "plan {plan:?}"),
            InputAction::Text(t) => assert!(!t.contains("this"), "plan {plan:?}"),
            _ => {}
        }
    }
    assert!(screen.ends_with("test "), "{screen:?}");
}

#[test]
fn contractions_still_restore_the_word_before_the_apostrophe() {
    let mut d = dispatcher();
    assert_eq!(replay(&mut d, "it's "), "it's ");
    let mut d = dispatcher();
    assert_eq!(replay(&mut d, "didn't "), "didn't ");
}
