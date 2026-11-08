//! Regression tests for Telex composition state desynchronization / hybrid
//! passthrough and V-C-V boundary corruption when typing after a completed
//! toned Vietnamese syllable.
//!
//! Shared from cross-engine Vietnamese IME test cases in VieLotus-IM:
//! https://github.com/stevedat/VieLotus-IM
//!
//! These cases previously reproduced engine bugs (hybrid `đuowcju` output,
//! silently dropped tone keys after `dd`, `viêtju`/`tiêngsu` corruption).
//! The engine now preserves the completed syllable and starts a new one.

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

// ------------------------------------------------------------------
// 1. Typing 'u' after "được" across Telex keystroke orders
// ------------------------------------------------------------------
//
// `is_raw_passthrough_slice` used to compare raw `'d'` against rendered `'đ'`,
// so `is_now_raw` stayed false for every `dd`-initial word and the V-C-V
// boundary split never ran — producing the hybrid `đuowcju`.

#[test]
fn post_toned_duoc_standard_order_plus_u() {
    assert_eq!(type_sequence("dduowcju"), "đượcu");
}

#[test]
fn post_toned_duoc_tone_before_coda_plus_u() {
    assert_eq!(type_sequence("dduowjcu"), "đượcu");
}

#[test]
fn post_toned_duoc_early_tone_on_u_plus_u() {
    assert_eq!(type_sequence("ddujowcu"), "đượcu");
}

// A tone key typed right after `dd` (before any vowel) is deferred as a
// pending tone: invisible until the first vowel adopts it, so "ddjuowc"
// becomes "được" instead of silently dropping the key ("đươc") or showing
// it literally. With no vowel ever arriving ("ddf"), it stays invisible
// and the word renders "đ" — the documented absorb contract.

#[test]
fn tone_key_immediately_after_dd_defers_to_first_vowel() {
    assert_eq!(type_sequence("ddjuowc"), "được");
    assert_eq!(type_sequence("ddjuowcu"), "đượcu");
    assert_eq!(type_sequence("ddju"), "đụ");
    assert_eq!(type_sequence("ddfu"), "đù");
    assert_eq!(type_sequence("ddsu"), "đú");
    // No vowel ever arrives: the pending tone is dropped, key stays hidden.
    assert_eq!(type_sequence("ddf"), "đ");
    assert_eq!(type_sequence("ddj"), "đ");
    // Tone-cancel keys keep the old absorb semantics (no pending).
    assert_eq!(type_sequence("ddz"), "đ");
    assert_eq!(type_sequence("ddzu"), "đu");
}

// ------------------------------------------------------------------
// 2. `dd`-initial words get the same optimistic-consonant and V-C-V
//    treatment as every other onset
// ------------------------------------------------------------------

#[test]
fn dd_initial_word_matches_non_dd_behavior() {
    // Optimistic consonant append: "nêb" -> "nêbo" on the trailing vowel.
    assert_eq!(type_sequence("neeb"), "nêb");
    assert_eq!(type_sequence("neebo"), "nêbo");
    assert_eq!(type_sequence("ddeeb"), "đêb");
    assert_eq!(type_sequence("ddeebo"), "đêbo");

    // V-C-V split: "nề" + "bo".
    assert_eq!(type_sequence("neefbo"), "nềbo");
    assert_eq!(type_sequence("ddeefbo"), "đềbo");

    // "ê" + 'b' is an invalid rhyme/coda pair, so both spellings pass
    // through raw — the engine must not invent "đềb".
    assert_eq!(type_sequence("neefb"), "neefb");
    assert_eq!(type_sequence("ddeefb"), "đeefb");
}

// ------------------------------------------------------------------
// 3. V-C-V `find_split_point` must not strip coda/tone keys from a
//    completed toned syllable ("việt" + 'u' used to give "viêtju")
// ------------------------------------------------------------------

#[test]
fn vcv_split_preserves_post_coda_toned_syllables() {
    assert_eq!(type_sequence("vieetj"), "việt");
    assert_eq!(type_sequence("vieetju"), "việtu");
    assert_eq!(type_sequence("tieengs"), "tiếng");
    assert_eq!(type_sequence("tieengsu"), "tiếngu");
}
