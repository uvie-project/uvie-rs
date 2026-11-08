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
    // "master" renders "mátẻ" - the split commits "má" and only "ter" stays
    // composing. Escape must restore the WHOLE word.
    let mut d = dispatcher();
    type_text(&mut d, "master");
    let plan = expect_consume(d.handle(KeyEvent::escape(), &Context::default()));
    assert_eq!(plan, vec![InputAction::Backspace(4), text("master")]);
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
