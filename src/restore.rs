//! Word-boundary "restore raw keystrokes" check (LabanKey-style).
//!
//! At the word boundary a token that is not a valid Vietnamese word must show
//! the literal keystrokes instead of the marks the pipeline applied to it
//! (`law` → `law`, not `lă`). A Vietnamese word token is one syllable, so a
//! rendered syllable that cannot be a word — an impossible or always-closed
//! rhyme left open, a vowel left inside the coda, a tone on a stop coda that
//! never takes it — is non-Vietnamese input.
//!
//! Pure string analysis: no engine state, no Telex/VNI specifics (raw modifier
//! digits never reach the render), and it only runs once per word commit, so
//! there is no per-keystroke cost.

use crate::tables::is_legal_nucleus;

/// Tone ids, matching `crate::tone`'s encoding (0 = no tone).
const TONE_HUYEN: u8 = 2;
const TONE_HOI: u8 = 3;
const TONE_NGA: u8 = 4;

/// Split a rendered vowel into (base letter, tone). `None` for anything that is
/// not a Vietnamese vowel (consonants, digits, other scripts).
fn split_tone(c: char) -> Option<(char, u8)> {
    const FORMS: [(char, [char; 6]); 12] = [
        ('a', ['a', 'á', 'à', 'ả', 'ã', 'ạ']),
        ('ă', ['ă', 'ắ', 'ằ', 'ẳ', 'ẵ', 'ặ']),
        ('â', ['â', 'ấ', 'ầ', 'ẩ', 'ẫ', 'ậ']),
        ('e', ['e', 'é', 'è', 'ẻ', 'ẽ', 'ẹ']),
        ('ê', ['ê', 'ế', 'ề', 'ể', 'ễ', 'ệ']),
        ('i', ['i', 'í', 'ì', 'ỉ', 'ĩ', 'ị']),
        ('o', ['o', 'ó', 'ò', 'ỏ', 'õ', 'ọ']),
        ('ô', ['ô', 'ố', 'ồ', 'ổ', 'ỗ', 'ộ']),
        ('ơ', ['ơ', 'ớ', 'ờ', 'ở', 'ỡ', 'ợ']),
        ('u', ['u', 'ú', 'ù', 'ủ', 'ũ', 'ụ']),
        ('ư', ['ư', 'ứ', 'ừ', 'ử', 'ữ', 'ự']),
        ('y', ['y', 'ý', 'ỳ', 'ỷ', 'ỹ', 'ỵ']),
    ];
    for (base, forms) in FORMS {
        if let Some(tone) = forms.iter().position(|&f| f == c) {
            return Some((base, tone as u8));
        }
    }
    None
}

/// Rhymes that are always closed: an open syllable with one of these nuclei is
/// not a Vietnamese word (`law` → `lă`, `wow` → `ươ`). `uơ` stays open (thuở,
/// huơ), and the open triphthongs (`ươi`, `ươu`, `uôi`, ...) are valid.
fn is_always_closed(nucleus: &[char]) -> bool {
    const CLOSED: [&[char]; 9] = [
        &['ă'],
        &['â'],
        &['i', 'ê'],
        &['y', 'ê'],
        &['u', 'ô'],
        &['ư', 'ơ'],
        &['u', 'â'],
        &['o', 'ă'],
        &['u', 'y', 'ê'],
    ];
    CLOSED.contains(&nucleus)
}

/// Returns `true` when a committed word's rendered syllable cannot be a
/// Vietnamese word, so the word boundary must restore the literal keystrokes.
///
/// `s` is the rendered text of the syllable (`prev_rendered`), any case.
pub(crate) fn rendered_syllable_needs_restore(s: &str) -> bool {
    // Split into lowercase (base, tone) letters. A syllable is short; a longer
    // render is not one and is left alone.
    let mut letters = ['\0'; 16];
    let mut tone = 0u8;
    let mut n = 0usize;
    for c in s.chars() {
        let mut lower = c.to_lowercase();
        let lc = lower.next().unwrap_or(c);
        if lower.next().is_some() {
            return false;
        }
        let base = match split_tone(lc) {
            Some((base, t)) => {
                if t != 0 {
                    tone = t;
                }
                base
            }
            // Consonants are ASCII letters plus đ; digits and other scripts
            // never form a Vietnamese syllable (VNI modifier keys are consumed
            // by the pipeline, so they do not appear in the render).
            None if lc.is_ascii_alphabetic() || lc == 'đ' => lc,
            None => return false,
        };
        if n == letters.len() {
            return false;
        }
        letters[n] = base;
        n += 1;
    }

    // onset | nucleus | coda
    let mut onset_end = 0usize;
    while onset_end < n && split_tone(letters[onset_end]).is_none() {
        onset_end += 1;
    }
    // `gi` and `qu` glides belong to the onset (già → g + a, quà → qu + a).
    if onset_end == 1 && onset_end + 1 < n {
        let first = letters[onset_end];
        let after_is_vowel = split_tone(letters[onset_end + 1]).is_some();
        if (letters[0] == 'g' && first == 'i' && after_is_vowel)
            || (letters[0] == 'q' && first == 'u')
        {
            onset_end += 1;
        }
    }
    let mut nucleus_end = onset_end;
    while nucleus_end < n && split_tone(letters[nucleus_end]).is_some() {
        nucleus_end += 1;
    }
    let nucleus = &letters[onset_end..nucleus_end];
    let coda = &letters[nucleus_end..n];

    // Teen-code abbreviations ("đc", "đt", "đk") have no nucleus.
    if nucleus.is_empty() {
        return false;
    }
    // A vowel after the nucleus means literal letters survived in the coda:
    // "díplay" → coda "play", "àtẻ" → coda "te".
    if coda.iter().any(|&c| split_tone(c).is_some()) {
        return true;
    }
    if !is_legal_nucleus(nucleus) {
        return true;
    }
    if coda.is_empty() && is_always_closed(nucleus) {
        return true;
    }
    // -c/-ch/-p/-t rhymes only take sắc/nặng; huyền/hỏi/ngã there is a tone key
    // the pipeline applied to a non-Vietnamese word ("after" → "àtẻ").
    let stop_coda = matches!(coda, ['c'] | ['c', 'h'] | ['p'] | ['t']);
    matches!(tone, TONE_HUYEN | TONE_HOI | TONE_NGA) && stop_coda
}

#[cfg(test)]
mod tests {
    use super::rendered_syllable_needs_restore as needs;

    #[test]
    fn restores_non_vietnamese_renders() {
        for s in [
            "lă",
            "să",
            "ươ",
            "dằ", // English + w/double-key renders
            "àtẻ",
            "ưatẻ",
            "sẻvẻ",
            "lẩvel",
            "díplay",
            "mátẻ",
            "chẩcter",
        ] {
            assert!(needs(s), "{s:?} must be restored");
        }
    }

    #[test]
    fn keeps_vietnamese_syllables() {
        for s in [
            "tiếng", "Việt", "người", "thuở", "huơ", "viêt", "ớ", "ừ", "ơi", "ủa", "Đắk", "hòa",
            "khuyến", "già", "quà", "lệch", "thuốc", "cháo", "uể",
        ] {
            assert!(!needs(s), "{s:?} must be kept");
        }
    }

    #[test]
    fn teen_code_abbreviations_are_kept() {
        for s in ["đc", "đt", "đk", "dc", ""] {
            assert!(!needs(s), "{s:?} must be kept");
        }
    }

    #[test]
    fn always_closed_rhymes_left_open_are_restored() {
        for s in ["ă", "â", "iê", "yê", "uô", "ươ", "uâ", "oă", "uyê", "LĂ"] {
            assert!(needs(s), "{s:?} must be restored");
        }
        // uơ is a valid open rhyme (thuở, huơ); the open triphthongs too.
        for s in ["uơ", "ơi", "ươi", "ươu", "uôi", "oai"] {
            assert!(!needs(s), "{s:?} must be kept");
        }
    }

    #[test]
    fn stop_coda_tone_rule() {
        assert!(needs("hòc"), "huyền + c");
        assert!(needs("hỏch"), "hỏi + ch");
        assert!(needs("hõp"), "ngã + p");
        assert!(needs("hìt"), "huyền + t");
        // nặng and sắc are the tones real -c/-ch/-p/-t words take.
        assert!(!needs("họt"));
        assert!(!needs("hót"));
        // Non-stop codas take every tone.
        assert!(!needs("hòm"));
        assert!(!needs("hòn"));
        assert!(!needs("hòa"));
    }

    #[test]
    fn non_letter_renders_are_kept() {
        assert!(!needs("a8"));
        assert!(!needs("lăw2"));
    }
}
