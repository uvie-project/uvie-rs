//! Vietnamese orthography accuracy tests.
//!
//! Grounded in standard Quốc Ngữ phonotactics:
//! - Valid rimes are the whitelisted diphthongs/triphthongs (ai, ao, au, âu,
//!   ây, eo, êu, iu, oi, ôi, ơi, ui, ưi, ưu, ia/iê/yê, ua/uô, ưa/ươ, oa, oă,
//!   oe, uâ, uê, uy + triphthongs). Non-standard rimes ("êo", "ôu", "ơu",
//!   "ưo", "io", "ău", "ăy") must NOT render as Vietnamese.
//! - A syllable boundary inside a written token must fall on a consonant
//!   (vowel-to-vowel hiatus never occurs inside a Vietnamese token).
//!
//! Sources: Vietnamese orthography (en.wikipedia.org/wiki/Vietnamese_orthography),
//! "Các vần trong tiếng Việt" (vi.wikipedia.org/wiki/Ngữ_âm_tiếng_Việt).

use uvie::UltraFastViEngine;
use uvie::diff::Diffable;

fn type_diff(e: &mut UltraFastViEngine, s: &str) -> String {
    let mut screen = String::new();
    for ch in s.chars() {
        let (bs, suffix) = e.feed_diff(ch);
        let screen_chars: Vec<char> = screen.chars().collect();
        let new_len = screen_chars.len().saturating_sub(bs);
        screen = screen_chars[..new_len].iter().collect::<String>();
        screen.push_str(suffix);
    }
    screen
}

#[test]
fn no_hyphen_hiatus_commits() {
    // Vowel-to-vowel hiatus ("thê"|"o") never occurs inside a Vietnamese
    // token — the engine must keep composing raw instead of committing the
    // fake word "thêo".
    let cases = [
        ("theeo", "theeo"),   // th + ee + o — must NOT become "thêo"
        ("keeo", "keeo"),     // must NOT become "kêo"
        ("neeabo", "neeabo"), // hiatus after cross-tone candidate
    ];
    for (input, expected) in cases {
        let mut e = UltraFastViEngine::new();
        assert_eq!(type_diff(&mut e, input), expected, "input {input}");
    }
}

#[test]
fn tone_cancel_then_english_word_passthrough() {
    // "vessarch": double-s cancels the sắc, then the word continues as
    // English. The V-C-V split must not resurrect the cancelled tone as a
    // committed "vế" syllable (hiatus split "vế"|"a" is illegal).
    // ("ressearch" can no longer be the vehicle: "ress" is an English
    // override word, so the dictionary fires before the cancel.)
    let mut e = UltraFastViEngine::new();
    assert_eq!(type_diff(&mut e, "vessarch"), "vesarch");
}

#[test]
fn nonstandard_rimes_passthrough() {
    // Rimes removed from the nucleus table (zero attestations in the 22k
    // word list, absent from the standard rime inventory).
    let cases = [
        ("keeo", "keeo"),   // êo
        ("kio", "kio"),     // io
        ("tawuf", "tawuf"), // ău
    ];
    for (input, expected) in cases {
        let mut e = UltraFastViEngine::new();
        assert_eq!(type_seq_passthrough(input), expected, "input {input}");
    }
}

fn type_seq_passthrough(s: &str) -> String {
    let mut e = UltraFastViEngine::new();
    for c in s.chars() {
        e.feed(c);
    }
    e.feed(' ');
    e.committed_text().trim_end().to_string()
}

#[test]
fn locked_vcv_behavior_unchanged() {
    // Consonant-onset V-C-V splits (the legitimate feature) must be intact.
    let cases = [
        ("neebo", "nêbo"),
        ("neeboo", "nêbô"),
        ("naabo", "nâbo"),
        ("toocaa", "tôcâ"),
        // double-s cancel: the cancelled s is a literal consonant, so
        // e+s+e must not bubble into ê with the sắc.
        ("vesset", "veset"),
        ("befe", "bề"), // cross-tone circumflex
    ];
    for (input, expected) in cases {
        let mut e = UltraFastViEngine::new();
        assert_eq!(type_diff(&mut e, input), expected, "input {input}");
    }
}

#[test]
fn uo_rhyme_tone_resolution() {
    // /uə/ rhyme: plain [u,o] + consonant coda + tone → the second vowel takes
    // the circumflex and carries the tone ("thuốc", "muốn", "nguồn", "thuộc").
    // Both keystroke orders must work.
    let cases = [
        ("thuocs", "thuốc"),
        ("muons", "muốn"),
        ("vuots", "vuốt"),
        ("suots", "suốt"),
        ("nguonf", "nguồn"),
        ("thuocj", "thuộc"),
        ("muonj", "muộn"),
        // Scrambled tone-key orders (data-driven pair encodings).
        ("thuocos", "thuốc"),
        ("muonos", "muốn"),
        ("thuocso", "thuốc"),
    ];
    for (input, expected) in cases {
        let mut e = UltraFastViEngine::new();
        assert_eq!(type_diff(&mut e, input), expected, "input {input}");
    }
}

#[test]
fn monophthong_coda_restrictions() {
    // Per-vowel coda rules: i/y take nh/ch (not ng/c), ơ only m n p t, etc.
    // Tone keys inside English words must not produce fake Vietnamese.
    let cases = [
        ("bings", "bings"),   // i+ng invalid
        ("myng", "myng"),     // y+ng invalid
        ("sowrng", "sowrng"), // ơ+ng invalid
        ("thuonh", "thuonh"), // uo+nh invalid
        // Growth intermediates stay typeable (c → ch).
        ("bicjh", "bịch"),
        ("bicsh", "bích"),
        ("becehj", "bệch"),
        // Real words with the restricted codas still work.
        ("tinhf", "tình"),  // i+nh, huyền ✓
        ("tinjh", "tịnh"),  // i+nh, nặng ✓
        ("lechf", "lechf"), // ê+ch with huyền: no such word → passthrough
        ("lechj", "lệch"),  // ê+ch, nặng ✓ (resolution e→ê)
        ("chowpj", "chợp"), // ơ+p, nặng ✓
    ];
    for (input, expected) in cases {
        let mut e = UltraFastViEngine::new();
        assert_eq!(type_diff(&mut e, input), expected, "input {input}");
    }
}
