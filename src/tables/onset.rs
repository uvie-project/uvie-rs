//! Onset (initial consonant cluster) validation.

/// Legal Vietnamese initial consonant clusters (raw ASCII).
///
/// The empty onset (word starts with a vowel) is handled by the caller.
/// Single-char consonants b/c/d/g/h/k/l/m/n/p/q/r/s/t/v/x are all legal;
/// they are listed here as 1-char entries. Multi-char clusters are explicit.
///
/// Standard Vietnamese consonant clusters:
/// ```text
/// {NGH}, {PH}, {TH}, {TR}, {GI}, {CH}, {NH}, {NG}, {KH}, {GH},
/// {G}, {C}, {Q}, {K}, {T}, {R}, {H}, {B}, {M}, {V}, {N}, {L},
/// {X}, {P}, {S}, {D}, (W as foreign/special; F/Z/J removed)
/// ```
pub(crate) static LEGAL_ONSETS: &[&[u8]] = &[
    // 3-char
    b"ngh", // 2-char
    b"ph", b"th", b"tr", b"gi", b"ch", b"nh", b"ng", b"kh", b"gh", b"qu",
    // 1-char (all legal single-consonant onsets)
    b"b", b"c", b"d", b"g", b"h", b"k", b"l", b"m", b"n", b"p", b"q", b"r", b"s", b"t", b"v", b"x",
    // đ (base 'd', but in practice the engine stores raw 'd' for đ onset too)
    b"d",
    // Foreign/extended allowed as onset
    // NOTE: 'f' removed to fix "fix" -> "fĩ", 'j'/'z' removed to fix
    // "jira" -> "jỉa" / "zoo" -> "zô" flicker (English word interference)
    b"w",
];

/// Onset↔nucleus compatibility.
///
/// Vietnamese has a hard orthographic distribution for the palatal/velar
/// consonant series — `c`/`k`/`q` and `g`/`gh`/`ng`/`ngh` are complementary:
///
/// | onset | allowed nucleus-first vowels (resolved)        |
/// |-------|-----------------------------------------------|
/// | `gh`  | `i`, `e`, `ê`                                 |
/// | `ngh` | `i`, `e`, `ê`                                 |
/// | `k`   | `i`, `e`, `ê`, `y`                            |
/// | `c`   | `a`, `ă`, `â`, `o`, `ô`, `ơ`, `u`, `ư`        |
/// | `g`   | `a`, `ă`, `â`, `o`, `ô`, `ơ`, `u`, `ư`, `i`¹  |
/// | `ng`  | `a`, `ă`, `â`, `o`, `ô`, `ơ`, `u`, `ư`        |
/// | `q`   | never valid alone (must form `qu` glide)²     |
///
/// ¹ `g` + `i` is the standalone `gi` syllable family (`gì`, `gíp`, `gịt`)
///   — the `gi` digraph onset is only split off by `partition_syllable` when
///   a vowel follows, so `g` with nucleus `i` here is exactly the bare `gi`.
/// ² When `q` is followed by `u`, `partition_syllable` extends the onset to
///   `qu`, so a length-1 `q` onset with a real nucleus means `q` was not
///   followed by `u` — always invalid.
///
/// `nucleus_first` is the **resolved** first nucleus vowel (e.g. `'ê'` for
/// `ế`, `'ơ'` for `ớ`), as returned by `Syl::base_no_tone()`.
///
/// All other onsets (`ch`, `tr`, `th`, `ph`, `kh`, `nh`, `b`, `d`, `đ`, `h`,
/// `l`, `m`, `n`, `p`, `r`, `s`, `t`, `v`, `x`, `qu`, `gi`, …) combine with
/// every legal nucleus, so this returns `true` for them.
pub fn onset_nucleus_compatible(onset: &[u8], nucleus_first: char) -> bool {
    match onset {
        b"gh" | b"ngh" => matches!(nucleus_first, 'i' | 'e' | 'ê'),
        b"k" => matches!(nucleus_first, 'i' | 'e' | 'ê' | 'y'),
        b"c" => !matches!(nucleus_first, 'i' | 'e' | 'ê' | 'y'),
        b"g" => matches!(
            nucleus_first,
            'a' | 'ă' | 'â' | 'o' | 'ô' | 'ơ' | 'u' | 'ư' | 'i'
        ),
        b"ng" => matches!(nucleus_first, 'a' | 'ă' | 'â' | 'o' | 'ô' | 'ơ' | 'u' | 'ư'),
        // length-1 `q` with a real nucleus: `q` wasn't followed by `u`.
        b"q" => false,
        _ => true,
    }
}

/// Returns `true` if `onset` (slice of raw base bytes before the nucleus) is
/// a legal Vietnamese initial cluster.  The empty onset is always legal.
pub fn is_legal_onset(onset: &[u8]) -> bool {
    match onset.len() {
        0 => true,
        1 => {
            let b = onset[0];
            // Any single lowercase consonant that is not a pure vowel key
            // NOTE: 'f' excluded to fix "fix" -> "fĩ", 'j'/'z' excluded to fix
            // "jira" -> "jỉa" / "zoo" -> "zô" (English word interference)
            matches!(
                b,
                b'b' | b'c'
                    | b'd'
                    | b'g'
                    | b'h'
                    | b'k'
                    | b'l'
                    | b'm'
                    | b'n'
                    | b'p'
                    | b'q'
                    | b'r'
                    | b's'
                    | b't'
                    | b'v'
                    | b'w'
                    | b'x'
            )
        }
        2 => {
            // Explicit 2-char whitelist
            matches!(
                onset,
                b"ph" | b"th" | b"tr" | b"gi" | b"ch" | b"nh" | b"ng" | b"kh" | b"gh" | b"qu"
            )
        }
        3 => onset == b"ngh",
        _ => false,
    }
}

/// Returns `true` if the given raw onset bytes form a **legal prefix** of some
/// Vietnamese onset - i.e., the onset is valid as-is OR could become valid with
/// more keystrokes.
///
/// Used to decide whether to keep composing or fall through to English
/// passthrough on the very first consonants.
pub fn is_onset_prefix(prefix: &[u8]) -> bool {
    if prefix.is_empty() {
        return true;
    }
    // Check exact match first.
    if is_legal_onset(prefix) {
        return true;
    }
    // Check if any legal onset starts with `prefix`.
    for onset in LEGAL_ONSETS {
        if onset.starts_with(prefix) {
            return true;
        }
    }
    false
}
