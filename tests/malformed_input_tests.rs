//! Regression tests for malformed / sloppy input handling through the diff
//! API — the path the macOS app actually drives. These lock in the
//! restore-to-raw behaviors a typing user relies on:
//!
//!  - Invalid Vietnamese must render the literal raw keystrokes (never a
//!    garbled hybrid, never a silently dropped character).
//!  - Cancel/absorb leniency is pinned: a repeated cancel key ('ss', 'ddd')
//!    or a stray vowel repeating a resolved circumflex source collapses
//!    exactly one key — the rest of the raw input survives.
//!  - Valid sloppy Telex must still resolve to the intended word.
//!  - A silently absorbed key stays in `word_raw`: typing past it rolls the
//!    word back to the literal passthrough — nothing is permanently lost.
//!
//! Expectations were captured by driving `feed_diff` + `commit_diff` exactly
//! as `EventTap` does (space → commit, then push the boundary char).

use uvie::UltraFastViEngine;
use uvie::diff::Diffable;

fn apply_diff(screen: &mut String, bs: usize, suffix: &str) {
    let sc: Vec<char> = screen.chars().collect();
    *screen = sc[..sc.len().saturating_sub(bs)].iter().collect::<String>();
    screen.push_str(suffix);
}

/// Type `input` through feed_diff; ' ' commits like the app's space path.
fn type_seq(input: &str) -> String {
    let mut engine = UltraFastViEngine::new();
    engine.set_modern_orthography(true);
    let mut screen = String::new();
    for ch in input.chars() {
        if ch == ' ' {
            let (bs, suffix) = engine.commit_diff();
            apply_diff(&mut screen, bs, &suffix);
            screen.push(' ');
            continue;
        }
        let (bs, suffix) = engine.feed_diff(ch);
        apply_diff(&mut screen, bs, &suffix);
    }
    screen
}

// ------------------------------------------------------------------
// Invalid Vietnamese restores the literal raw keystrokes
// ------------------------------------------------------------------

#[test]
fn malformed_restores_literal() {
    // Overlong nuclei, impossible onsets, stray tone keys — every one of
    // these must render exactly what was typed.
    for (input, expected) in [
        ("shoa ", "shoa "),
        ("fhoa ", "fhoa "),
        ("duocsf ", "duocsf "),
        ("hoaz ", "hoaz "),
        ("bge ", "bge "),
        ("gge ", "gge "),
        ("chuowg ", "chuowg "),
        ("uowjcf ", "uowjcf "),
        ("doung ", "doung "),
        ("xongg ", "xongg "),
        ("xogng ", "xogng "),
        ("hsoaf ", "hsoaf "),
        ("uyenjt ", "uyenjt "),
        ("bookes ", "bookes "),
        ("feelix ", "feelix "),
        ("thuongse ", "thuongse "),
        ("chuongse ", "chuongse "),
        // Bare modifier/tone keys with nothing to modify pass through.
        ("z ", "z "),
        ("s ", "s "),
    ] {
        assert_eq!(type_seq(input), expected, "input: {:?}", input);
    }
}

// ------------------------------------------------------------------
// Cancel / absorb leniency — one key collapses, the rest survive
// ------------------------------------------------------------------

#[test]
fn malformed_cancel_leniency() {
    for (input, expected) in [
        // Double same tone key cancels: keeps one literal key.
        ("hoass ", "hoas "),
        // 'ddd' cancels the đ (pinned dd-cancel semantics).
        ("ddd ", "dd "),
        ("dddd ", "ddd "),
        ("ddduoc ", "dduoc "),
        ("dddon ", "ddon "),
        // A stray repeat of a resolved circumflex source vowel collapses.
        ("nghieeengs ", "nghieengs "),
        ("vieeejt ", "vieejt "),
        ("tueeen ", "tueen "),
        ("tooof ", "tòo "),
    ] {
        assert_eq!(type_seq(input), expected, "input: {:?}", input);
    }
}

// ------------------------------------------------------------------
// Sloppy-but-valid Telex still resolves
// ------------------------------------------------------------------

#[test]
fn sloppy_valid_telex_resolves() {
    for (input, expected) in [
        ("thuwong ", "thương "),
        ("duowng ", "dương "),
        ("duowngf ", "dường "),
        ("muojng ", "muộng "),
        ("hoanfg ", "hoàng "),
        ("hoasf ", "hoà "),
        ("hosaf ", "hoà "),
        ("dangfs ", "dáng "),
        ("vietsj ", "việt "),
        ("ieen ", "iên "),
        ("ieenj ", "iện "),
        ("kieeur ", "kiểu "),
        ("xongf ", "xòng "),
        ("nghiemx ", "nghiễm "),
        ("khoawng ", "khoăng "),
        ("khawng ", "khăng "),
        ("giuwxng ", "giững "),
        ("uowjc ", "ược "),
        ("quuxng ", "quũng "),
        ("cuofi ", "cuồi "),
        // Bare modifier keys still apply when a target exists.
        ("aw ", "ă "),
        ("w ", "ư "),
        ("uw ", "ư "),
        ("ow ", "ơ "),
    ] {
        assert_eq!(type_seq(input), expected, "input: {:?}", input);
    }
}

// ------------------------------------------------------------------
// Toned-circumflex absorb: stray vowel is held, not lost
// ------------------------------------------------------------------

#[test]
fn absorb_holds_stray_vowel_then_rolls_back() {
    // "nghiếng" + 'e': the extra 'e' repeats the circumflex source vowel of
    // a resolved toned rhyme — absorbed for leniency ("thuocso" semantics,
    // pinned by the 30k telex-pairs corpus).
    assert_eq!(type_seq("nghiengse "), "nghiếng ");
    assert_eq!(type_seq("tiengse "), "tiếng ");
    assert_eq!(type_seq("vieengse "), "viếng ");
    assert_eq!(type_seq("thuocso "), "thuốc ");

    // The absorbed key stays in word_raw: typing on past it makes the word
    // invalid again and the whole word rolls back to literal passthrough —
    // the 'e' is never permanently lost.
    assert_eq!(type_seq("nghieengses "), "nghieengse ");
    assert_eq!(type_seq("nghieengsea "), "nghieengsea ");
}

// ------------------------------------------------------------------
// V-C-V prefix commits are irreversible (documented limit)
// ------------------------------------------------------------------

#[test]
fn vcv_prefix_commit_is_final() {
    // Once a V-C-V split commits a Vietnamese-looking prefix, the tail
    // cannot un-transform it — the word stays half-Vietnamese. This is a
    // documented design limit (the prefix is already on screen).
    assert_eq!(type_seq("neeboxyz "), "nêboxyz ");
    assert_eq!(type_seq("meexyz "), "mêxyz ");
    assert_eq!(type_seq("seejing "), "sêjing ");
}

// ------------------------------------------------------------------
// Silent consume inventory — whitelist only the documented absorbs
// ------------------------------------------------------------------

/// Any keystroke that returns (0, "") while the engine is composing looks
/// dead to the user — the app consumes the event and nothing renders. The
/// only sanctioned cases are the documented `w` silent-consume (all horn
/// candidates invalid) and the toned-circumflex vowel absorb; anything
/// else is a dead-key regression.
#[test]
fn no_silent_consumes_outside_documented_absorbs() {
    let mut hits: Vec<String> = Vec::new();
    let prefixes = ["", "t", "th", "nh", "ng", "ch", "d", "dd", "ngh", "v"];
    let nuclei = [
        "a", "aa", "o", "oo", "u", "uo", "ow", "uw", "aw", "ie", "i", "eng", "ong",
    ];
    let tails = [
        "s", "f", "r", "x", "j", "z", "ss", "a", "e", "o", "i", "u", "se", "sf",
    ];
    for p in prefixes {
        for n in nuclei {
            for t in tails {
                let seq = format!("{}{}{}", p, n, t);
                let mut e = UltraFastViEngine::new();
                for ch in seq.chars() {
                    let (bs, suf) = e.feed_diff(ch);
                    let empty = suf.is_empty();
                    let composing = e.is_composing_diff();
                    if bs == 0 && empty && composing {
                        hits.push(format!("{} @{}", seq, ch));
                    }
                }
            }
        }
    }
    assert!(hits.is_empty(), "undocumented silent consumes: {:?}", hits);
}
