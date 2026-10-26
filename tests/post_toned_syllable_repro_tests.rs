//! Reproduction tests for Telex composition state desynchronization / hybrid passthrough
//! and V-C-V boundary corruption when typing after a completed toned Vietnamese syllable.
//!
//! Shared from cross-engine Vietnamese IME test cases in VieLotus-IM:
//! https://github.com/stevedat/VieLotus-IM

use std::panic::{AssertUnwindSafe, catch_unwind};
use uvie::UltraFastViEngine;
use uvie::diff::Diffable;

fn apply_diff(screen: &mut String, bs: usize, suffix: &str) {
    let sc: Vec<char> = screen.chars().collect();
    *screen = sc[..sc.len().saturating_sub(bs)].iter().collect::<String>();
    screen.push_str(suffix);
}

fn type_sequence(input: &str) -> String {
    let mut engine = UltraFastViEngine::new();
    engine.set_modern_orthography(true);
    let mut screen = String::new();
    for ch in input.chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }
    screen
}

/// Equivalent to `XCTExpectFailure` in Swift XCTest: verifies that the desired assertion
/// currently fails due to the documented bug (so CI stays green while reproducing the issue,
/// and alerts when the underlying bug is fixed).
fn expect_known_bug(reason: &str, f: impl FnOnce()) {
    let result = catch_unwind(AssertUnwindSafe(f));
    assert!(
        result.is_err(),
        "Expected known bug ({reason}) to fail assertion, but it passed! Remove `expect_known_bug` wrapper."
    );
}

// ------------------------------------------------------------------
// 1. Typing 'u' after "được" across Telex keystroke orders
// ------------------------------------------------------------------
//
// Root cause in `src/composing.rs` (`render_passthrough`) & `src/diff/utils.rs`
// (`is_raw_passthrough_slice`):
// - When a word starting with `dd` (`đ`) becomes invalid Vietnamese, `render_passthrough`
//   preserves `'đ'` (`\u{0111}`) for `dd` while emitting raw ASCII for the rest of `self.raw`
//   (producing the half-Vietnamese / half-raw string `"đuowcju"`).
// - Furthermore, `is_raw_passthrough_slice(&self.diff.raw_chars, &new_composed)` compares
//   `raw_chars[0]` (`'d'`) with `new_composed`'s first char (`'đ'`), returning `false` for
//   EVERY `dd`-initial word! As a result, `is_now_raw` in `feed_diff` (`src/diff/core.rs`)
//   is always `false` for `dd` words, overwriting `last_valid_out` and bypassing both
//   optimistic consonant append and V-C-V boundary splitting.

#[test]
fn repro_post_toned_duoc_standard_order_plus_u() {
    let actual = type_sequence("dduowcju");
    // Currently produces hybrid `"đuowcju"` (bs=3, out="uowcju" after "được").
    assert_eq!(actual, "đuowcju");

    expect_known_bug(
        "dduowcju unfolds vowels/tone into half-converted 'đuowcju'",
        || {
            assert!(
                actual == "đượcu" || actual == "dduowcju",
                "Expected either preserved syllable 'đượcu' or full raw 'dduowcju', got {actual:?}"
            );
        },
    );
}

#[test]
fn repro_post_toned_duoc_tone_before_coda_plus_u() {
    let actual = type_sequence("dduowjcu");
    // Currently produces hybrid `"đuowjcu"`.
    assert_eq!(actual, "đuowjcu");

    expect_known_bug(
        "dduowjcu unfolds vowels/tone into half-converted 'đuowjcu'",
        || {
            assert!(
                actual == "đượcu" || actual == "dduowjcu",
                "Expected either preserved syllable 'đượcu' or full raw 'dduowjcu', got {actual:?}"
            );
        },
    );
}

#[test]
fn repro_post_toned_duoc_early_tone_on_u_plus_u() {
    let actual = type_sequence("ddujowcu");
    // Currently produces hybrid `"đujowcu"`.
    assert_eq!(actual, "đujowcu");

    expect_known_bug(
        "ddujowcu unfolds vowels/tone into half-converted 'đujowcu'",
        || {
            assert!(
                actual == "đượcu" || actual == "ddujowcu",
                "Expected either preserved syllable 'đượcu' or full raw 'ddujowcu', got {actual:?}"
            );
        },
    );
}

#[test]
fn repro_post_toned_duoc_tone_immediately_after_dd() {
    // In `src/tone_handler.rs` (`handle_tone_key`), when `carrier.is_none()` and
    // `has_modified_consonant` is true (`dd` already formed `'đ'`), typing `'j'` before the
    // vowel decrements `self.raw_len -= 1` and silently drops `'j'`.
    let before_u = type_sequence("ddjuowc");
    assert_eq!(before_u, "đươc");

    let after_u = type_sequence("ddjuowcu");
    assert_eq!(after_u, "đuowcu");

    expect_known_bug(
        "ddjuowc silently drops 'j' tone key after 'dd' and unfolds on trailing 'u'",
        || {
            assert_eq!(before_u, "được");
            assert!(after_u == "đượcu" || after_u == "ddjuowcu");
        },
    );
}

// ------------------------------------------------------------------
// 2. `is_raw_passthrough_slice` always returns `false` for `dd` (`đ`) words,
//    breaking optimistic consonant append and V-C-V syllable splitting
// ------------------------------------------------------------------

#[test]
fn repro_dd_initial_word_breaks_optimistic_consonant_and_vcv_split() {
    // Compare with non-`dd` word `"neeb"` -> `"nêb"` and `"neebo"` -> `"nêbo"` in `vcv_tests.rs`:
    assert_eq!(type_sequence("neeb"), "nêb");
    assert_eq!(type_sequence("neebo"), "nêbo");

    // Because `is_raw_passthrough_slice` compares `'d'` with `'đ'`, `is_now_raw` is `false`
    // for `"ddeefb"` (`"đề"` + `'b'`) and `"ddeefbo"` (`"đề"` + `"bo"`):
    let actual_consonant = type_sequence("ddeefb");
    let actual_vcv = type_sequence("ddeefbo");
    assert_eq!(actual_consonant, "đeefb");
    assert_eq!(actual_vcv, "đeefbo");

    expect_known_bug(
        "is_raw_passthrough_slice returns false for 'dd' words, breaking V-C-V split",
        || {
            assert_eq!(actual_consonant, "đềb");
            assert_eq!(actual_vcv, "đềbo");
        },
    );
}

// ------------------------------------------------------------------
// 3. V-C-V `find_split_point` strips post-coda tone keys and corrupts
//    completed toned syllables ("việt" + 'u' -> "viêtju")
// ------------------------------------------------------------------
//
// Root cause in `src/diff/utils.rs` (`find_split_point`):
// - For `"vieetju"` (`"việt"` + `'u'`) or `"tieengsu"` (`"tiếng"` + `'u'`), the non-vowel
//   cluster between the last vowel and `'u'` is `['t', 'j']` or `['n', 'g', 's']`.
// - Because no suffix ending with tone modifier `'j'` / `'s'` satisfies `is_legal_onset`,
//   `find_split_point` falls through to `return run_start;`, splitting BEFORE the coda `'t'` / `"ng"`.
// - Syllable 1 loses both its coda and its tone (`"viê"` / `"tiê"`), while syllable 2 gets
//   coda + raw tone key + `'u'` (`"tju"` / `"ngsu"`), mutating `"việt"` -> `"viêtju"` and
//   `"tiếng"` -> `"tiêngsu"`.

#[test]
fn repro_vcv_split_corrupts_post_coda_toned_syllables() {
    let viet_u = type_sequence("vieetju");
    let tieng_u = type_sequence("tieengsu");
    assert_eq!(viet_u, "viêtju");
    assert_eq!(tieng_u, "tiêngsu");

    expect_known_bug(
        "find_split_point splits before coda when tone key follows coda, producing 'viêtju' / 'tiêngsu'",
        || {
            assert!(
                viet_u == "việtu" || viet_u == "vieetju",
                "Expected 'việtu' or 'vieetju', got {viet_u:?}"
            );
            assert!(
                tieng_u == "tiếngu" || tieng_u == "tieengsu",
                "Expected 'tiếngu' or 'tieengsu', got {tieng_u:?}"
            );
        },
    );
}
