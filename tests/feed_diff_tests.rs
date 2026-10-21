//! Integration tests for UltraFastViEngine::feed_diff - diff-based API.

use uvie::UltraFastViEngine;
use uvie::diff::Diffable;

fn apply_diff(screen: &mut String, bs: usize, suffix: &str) {
    let sc: Vec<char> = screen.chars().collect();
    *screen = sc[..sc.len().saturating_sub(bs)].iter().collect::<String>();
    screen.push_str(suffix);
}

fn assert_diff(input: &str, expected: &str) {
    let mut engine = UltraFastViEngine::new();
    engine.set_modern_orthography(true);
    let mut screen = String::new();
    for ch in input.chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }
    assert_eq!(screen, expected, "input: {:?}", input);
}

// ------------------------------------------------------------------
// Basic words
// ------------------------------------------------------------------

#[test]
fn diff_simple_word() {
    assert_diff("xin", "xin");
}

#[test]
fn diff_tone_mark() {
    assert_diff("tooi", "tôi");
}

#[test]
fn diff_space_boundary() {
    assert_diff("xin chao", "xin chao");
}

// ------------------------------------------------------------------
// Complex syllables
// ------------------------------------------------------------------

#[test]
fn diff_diphthong_tone() {
    assert_diff("toanj", "toạn");
}

#[test]
fn diff_diphthong_ng() {
    assert_diff("thuowngj", "thượng");
}

#[test]
fn diff_medial_o() {
    assert_diff("ngoanf", "ngoàn");
}

// ------------------------------------------------------------------
// Rollback / invalid
// ------------------------------------------------------------------

#[test]
fn diff_invalid_sequence() {
    assert_diff("fgh", "fgh");
}

// ------------------------------------------------------------------
// Backspace
// ------------------------------------------------------------------

#[test]
fn diff_backspace() {
    let mut engine = UltraFastViEngine::new();
    let mut screen = String::new();

    for ch in "tooi".chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }
    assert_eq!(screen, "tôi");

    let (bs, suffix) = engine.backspace_diff();
    apply_diff(&mut screen, bs, suffix);
    assert_eq!(screen, "tô");

    let (bs, suffix) = engine.backspace_diff();
    apply_diff(&mut screen, bs, suffix);
    assert_eq!(screen, "to");

    let (bs, suffix) = engine.backspace_diff();
    apply_diff(&mut screen, bs, suffix);
    assert_eq!(screen, "t");

    let (bs, suffix) = engine.backspace_diff();
    apply_diff(&mut screen, bs, suffix);
    assert_eq!(screen, "");
}

// ------------------------------------------------------------------
// Punctuation / word boundaries
// ------------------------------------------------------------------

#[test]
fn diff_comma_boundary() {
    assert_diff("xin,", "xin,");
}

#[test]
fn diff_period_boundary() {
    assert_diff("xin.", "xin.");
}

// ------------------------------------------------------------------
// V-C-V boundary detection
// ------------------------------------------------------------------

#[test]
fn diff_vcv_neebo() {
    let mut engine = UltraFastViEngine::new();
    let mut screen = String::new();
    for ch in "neebo".chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }
    assert_eq!(screen, "nêbo");
    assert_eq!(engine.committed_text_diff(), "nê");
}

#[test]
fn diff_vcv_neeboo() {
    let mut engine = UltraFastViEngine::new();
    let mut screen = String::new();
    for ch in "neeboo".chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }
    assert_eq!(screen, "nêbô");
    assert_eq!(engine.committed_text_diff(), "nê");
}

#[test]
fn diff_no_spurious_commit_tooi() {
    let mut engine = UltraFastViEngine::new();
    let mut screen = String::new();
    for ch in "tooi".chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }
    assert_eq!(screen, "tôi");
    assert_eq!(engine.committed_text_diff(), "");
}

#[test]
fn diff_fast_typing_ddau() {
    assert_diff("ddaau", "đâu");
}

// ------------------------------------------------------------------
// Backspace + retype scenarios
// ------------------------------------------------------------------

#[test]
fn diff_backspace_retype_thajta() {
    let mut engine = UltraFastViEngine::new();
    let mut screen = String::new();

    for ch in "thajta".chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }
    assert_eq!(screen, "thật");

    // BS once
    let (bs, suffix) = engine.backspace_diff();
    apply_diff(&mut screen, bs, suffix);
    assert_eq!(screen, "thạt");

    // Type 'a' again
    let (bs, suffix) = engine.feed_diff('a');
    apply_diff(&mut screen, bs, suffix);
    assert_eq!(screen, "thật");
}

#[test]
fn diff_backspace_clear_retype() {
    let mut engine = UltraFastViEngine::new();
    let mut screen = String::new();

    for ch in "thajta".chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }
    assert_eq!(screen, "thật");

    // BS 6 times to clear
    for _ in 0..6 {
        let (bs, suffix) = engine.backspace_diff();
        apply_diff(&mut screen, bs, suffix);
    }
    assert_eq!(screen, "");

    // Retype fresh
    for ch in "thajta".chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }
    assert_eq!(screen, "thật");
}

#[test]
fn diff_sticky_after_backspace() {
    let mut engine = UltraFastViEngine::new();
    let mut screen = String::new();

    for ch in "thajt".chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }

    let (bs, suffix) = engine.feed_diff('a');
    apply_diff(&mut screen, bs, suffix);
    assert_eq!(screen, "thật");

    // BS once -> "thạt"
    let (bs, suffix) = engine.backspace_diff();
    apply_diff(&mut screen, bs, suffix);
    assert_eq!(screen, "thạt");

    // Type 'a' again -> "thật"
    let (bs, suffix) = engine.feed_diff('a');
    apply_diff(&mut screen, bs, suffix);
    assert_eq!(screen, "thật");
}

// ------------------------------------------------------------------
// gif / tim sequences
// ------------------------------------------------------------------

#[test]
fn diff_gif() {
    assert_diff("gif", "gì");
}

#[test]
fn diff_tim_sequences() {
    assert_diff("timf", "tìm");
    assert_diff("tim", "tim");
}

// ------------------------------------------------------------------
// Multiple syllables
// ------------------------------------------------------------------

#[test]
fn diff_boo_boo() {
    assert_diff("boo boo", "bô bô");
}

// ------------------------------------------------------------------
// English passthrough
// ------------------------------------------------------------------

#[test]
fn diff_english_clear() {
    assert_diff("clear", "clear");
}

#[test]
fn diff_english_blob() {
    assert_diff("blob", "blob");
}

// ------------------------------------------------------------------
// Double-cancel
// ------------------------------------------------------------------

#[test]
fn diff_double_tone_cancel() {
    assert_diff("vess", "ves");
    assert_diff("teff", "tef");
    // Words ending in a doubled tone key are only kept in the English
    // override dict when they are common (top ~10k of the corpus): "pass"
    // stays literal, but rare "tess" falls through to the standard cancel
    // and collapses to "tes".
    assert_diff("pass", "pass");
    assert_diff("tess", "tes");
    assert_diff("cass", "cas");
}

#[test]
fn diff_double_tone_cancel_survives_dictionary_commit() {
    // A dictionary word ending in a doubled tone key ("mass") commits the
    // literal raw, but the pipeline's own raw is the collapsed form ("mas").
    // The commit must not pin the rest of the word to raw: a consonant tail
    // that cannot become a dictionary word resumes Vietnamese processing, so
    // the cancel keeps working for every such word.
    assert_diff("masster", "master");
    // A bare consonant tail carries no Vietnamese transform, so the literal
    // keystrokes stay until the tail reads as Vietnamese ("masste" → the
    // tone in "ter" is what re-opens the word).
    assert_diff("masst", "masst");
    assert_diff("masste", "masste");
    assert_diff("passter", "paster");
    assert_diff("misster", "mister");
    assert_diff("messter", "mester");
    assert_diff("tosster", "toster");
    // Word boundaries commit the same way.
    assert_diff("masster ", "master ");
    assert_diff("masster.", "master.");
}

#[test]
fn diff_double_tone_cancel_keeps_dictionary_words() {
    // The dictionary still protects the words themselves and every word
    // they can grow into — only a non-dictionary consonant tail re-opens
    // the cancel.
    assert_diff("mass", "mass");
    assert_diff("pass", "pass");
    assert_diff("boss", "boss");
    assert_diff("less", "less");
    assert_diff("massive", "massive");
    assert_diff("massively", "massively");
    assert_diff("masses", "masses");
    assert_diff("lesser", "lesser");
    assert_diff("passing", "passing");
    assert_diff("passport", "passport");
    assert_diff("password", "password");
    assert_diff("passerby", "passerby");
    assert_diff("correctness", "correctness");
    // English compounds whose tail stays literal are English intent — the
    // cancel only re-opens the word when the tail reads as Vietnamese.
    assert_diff("passkey", "passkey");
    assert_diff("passcode", "passcode");
    assert_diff("passphrase", "passphrase");
    assert_diff("passthrough", "passthrough");
    assert_diff("misspeak", "misspeak");
    assert_diff("misspelt", "misspelt");
    assert_diff("misspend", "misspend");
    assert_diff("mossback", "mossback");
}

#[test]
fn diff_double_tone_cancel_backspace_after_fallback() {
    // Once the cancel is re-opened the word walks back like any composing
    // word ("masster" → "maste" → "mas"), and erasing the whole word leaves
    // fresh Vietnamese typing intact.
    assert_eq!(diff_type_backspace("masster", 1), "maste");
    assert_eq!(diff_type_backspace("masster", 3), "mas");
    assert_eq!(diff_type_backspace("masster", 7), "");
}

#[test]
fn diff_double_tone_cancel_backspace_past_dictionary_word() {
    // Backspacing below the dictionary word the fallback was armed for must
    // drop the saved state: restoring it would rewrite a word the user has
    // already deleted ("mass" ⌫ "xt" must stay "masxt", not resurrect "mass").
    let mut engine = UltraFastViEngine::new();
    engine.set_modern_orthography(true);
    let mut screen = String::new();
    for ch in "mass".chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }
    assert_eq!(screen, "mass");
    let (bs, suffix) = engine.backspace_diff();
    apply_diff(&mut screen, bs, suffix);
    assert_eq!(screen, "mas");
    for ch in "xter".chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }
    assert_eq!(engine.word_raw_string().to_string(), "masxter");
    assert!(
        screen.starts_with("masx"),
        "the deleted dictionary word came back: {screen:?}"
    );
    assert_eq!(screen, engine.current_output_diff().to_string());
}

#[test]
fn diff_double_w_cancel() {
    assert_diff("showw", "show");
    assert_diff("oww", "ow");
}

// ------------------------------------------------------------------
// dd/ww cancel
// ------------------------------------------------------------------

#[test]
fn diff_dd_cancel() {
    assert_diff("ddd", "dd");
}

#[test]
fn diff_ww_cancel() {
    assert_diff("ww", "w");
    assert_diff("wwork", "work");
}

// ------------------------------------------------------------------
// Sticky English passthrough (prefix override + raw tail)
// ------------------------------------------------------------------

/// Type `input`, then press backspace `bs_count` times. Returns the screen.
fn diff_type_backspace(input: &str, bs_count: usize) -> String {
    let mut engine = UltraFastViEngine::new();
    engine.set_modern_orthography(true);
    let mut screen = String::new();
    for ch in input.chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }
    for _ in 0..bs_count {
        let (bs, suffix) = engine.backspace_diff();
        apply_diff(&mut screen, bs, suffix);
    }
    screen
}

#[test]
fn diff_sticky_prefix_override_tail_raw() {
    // "perm" fires the dictionary override mid-word; the tail must pass
    // through raw instead of re-transforming as Vietnamese (which produced
    // "permision" — the double-s cancel ate a letter).
    assert_diff("permission", "permission");
    // "syst" fires; without sticky the tail "ems" rendered "ém" → "systém".
    assert_diff("systems", "systems");
    // "afric" fires; tail "ans" would render "án" → "africán".
    assert_diff("africans", "africans");
    // Word-boundary commit after sticky keeps the raw word.
    assert_diff("permissions today", "permissions today");
}

#[test]
fn diff_sticky_backspace_walk() {
    // Backspace pops the raw tail first, then the committed prefix, one
    // char at a time.
    assert_eq!(diff_type_backspace("permission", 3), "permiss");
    assert_eq!(diff_type_backspace("permission", 6), "perm");
    assert_eq!(diff_type_backspace("permission", 10), "");
    // Partial erase then retype stays in passthrough.
    let mut engine = UltraFastViEngine::new();
    engine.set_modern_orthography(true);
    let mut screen = String::new();
    for ch in "permx".chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }
    let (bs, suffix) = engine.backspace_diff();
    apply_diff(&mut screen, bs, suffix);
    for ch in "its".chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }
    assert_eq!(screen, "permits");
}

#[test]
fn diff_sticky_full_erase_restores_vietnamese() {
    // Erasing the whole word (tail + prefix) leaves sticky mode: fresh
    // typing is Vietnamese again.
    let mut engine = UltraFastViEngine::new();
    engine.set_modern_orthography(true);
    let mut screen = String::new();
    for ch in "permission".chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }
    assert_eq!(screen, "permission");
    for _ in 0..10 {
        let (bs, suffix) = engine.backspace_diff();
        apply_diff(&mut screen, bs, suffix);
    }
    assert_eq!(screen, "");
    for ch in "vieetj".chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }
    assert_eq!(screen, "việt");
}

#[test]
fn diff_english_override_disabled() {
    // With the override off, dict words transform like any input.
    let mut engine = UltraFastViEngine::new();
    engine.set_modern_orthography(true);
    engine.set_english_override(false);
    let mut screen = String::new();
    for ch in "good".chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }
    assert_eq!(screen, "gôd");
}
// ------------------------------------------------------------------
// Raw keystrokes of the whole word (Escape restores literal input)
// ------------------------------------------------------------------

/// Type `input` and return (on-screen text, raw keystrokes of the word).
fn diff_type_screen_raw(input: &str) -> (String, String) {
    let mut engine = UltraFastViEngine::new();
    engine.set_modern_orthography(true);
    let mut screen = String::new();
    for ch in input.chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }
    (screen, engine.word_raw_string().to_string())
}

#[test]
fn diff_word_raw_spans_the_whole_word() {
    // A host "escape restores the literal keystrokes" action needs the raw
    // keys of the ENTIRE word, not just the composing syllable: the V-C-V
    // split commits "má" and leaves only "ter" composing, but the user
    // typed "master".
    let (screen, raw) = diff_type_screen_raw("master");
    assert_eq!(screen, "mátẻ");
    assert_eq!(raw, "master");

    // Same across a dictionary commit: the override commits "mass" and
    // passes the tail through raw.
    let (screen, raw) = diff_type_screen_raw("mass");
    assert_eq!(screen, "mass");
    assert_eq!(raw, "mass");

    // And after the cancel fallback.
    let (screen, raw) = diff_type_screen_raw("masster");
    assert_eq!(screen, "master");
    assert_eq!(raw, "masster");

    // Two-syllable V-C-V words keep every keystroke too.
    let (screen, raw) = diff_type_screen_raw("neebo");
    assert_eq!(screen, "nêbo");
    assert_eq!(raw, "neebo");
}

#[test]
fn diff_word_raw_empty_after_raw_buffer_overflow() {
    // A word longer than the 24-keystroke raw buffer commits its prefix
    // mid-word; the raw no longer covers the rendered text, so the host must
    // not restore it (an empty buffer means "cannot restore").
    let mut engine = UltraFastViEngine::new();
    engine.set_modern_orthography(true);
    let mut screen = String::new();
    for ch in "abcdefghijklmnopqrstuvwxyz".chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, suffix);
    }
    assert!(engine.is_composing_diff());
    assert!(!screen.is_empty());
    assert_eq!(engine.word_raw_string().to_string(), "");
}

#[test]
fn diff_word_raw_clears_on_commit() {
    let mut engine = UltraFastViEngine::new();
    engine.set_modern_orthography(true);
    for ch in "master".chars() {
        engine.feed_diff(ch);
    }
    assert_eq!(engine.word_raw_string().to_string(), "master");
    engine.commit_diff();
    assert_eq!(engine.word_raw_string().to_string(), "");
    assert!(!engine.is_composing_diff());
}
