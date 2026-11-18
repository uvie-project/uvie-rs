//! Double-vowel circumflex target lookup (used by the composing layer to detect
//! incomplete circumflex nuclei like `iê`, `yê`, `uê`, `â`, `ô`).

use crate::engine::UltraFastViEngine;
use crate::modes::IS_TONE_KEY;
use crate::syllable::{F_HORN, F_LITERAL};

/// Double-vowel circumflex target resolution.
pub(crate) trait DoubleVowelLookup {
    fn find_modifier_target_for_double_vowel(&self, b: u8) -> Option<usize>;
}

impl DoubleVowelLookup for UltraFastViEngine {
    #[inline]
    fn find_modifier_target_for_double_vowel(&self, b: u8) -> Option<usize> {
        if self.raw_len >= 3 {
            let prev = self.raw[self.raw_len - 2];
            let prev2 = self.raw[self.raw_len - 3];
            if self.mode.classify[prev as usize] & IS_TONE_KEY != 0 && prev2 == b {
                // Allow the tone key to sit between the two halves of an
                // incomplete circumflex nucleus:
                // - iê / yê / uê: i/y/u + e + tone + e (e.g. `ieje` -> iệ)
                // - ê:            e + tone + e         (e.g. `eje` -> ệ)
                // - â:            a + tone + a         (e.g. `aja` -> ậ)
                // - ô:            o + tone + o         (e.g. `ojo` -> ộ)
                if b == b'e' {
                    // For ê, allow any onset (bare, glide, or consonant).
                    // Previously, consonant onsets were blocked to avoid conflicts
                    // with English words like "reset". However, this also blocked
                    // valid Vietnamese words like "befe" → "bề", "biense" → "biến".
                    // The trade-off is that some English words with e+tone+e
                    // patterns (e.g. "reset" → "rết") will be transformed, but
                    // this affects far fewer cases than the Vietnamese words it
                    // fixes. The final is_valid_vietnamese() check in
                    // render_out_buf still catches most invalid cases.
                    // raw_len == 3: bare "e + tone + e" — allowed, fall through.
                } else if b != b'a' && b != b'o' {
                    return None;
                }
                // For a/o/e (bare), the doubled vowel itself is the nucleus —
                // no glide prefix required, so fall through to the normal search.
            }
        }

        let n = self.buf.len();
        for i in (0..n).rev() {
            let s = self.buf.get(i);
            // A cancelled tone key is a literal consonant: the vowel on the
            // other side of it is a separate letter, not the second half of a
            // circumflex pair ("resse" → "rese", "toffo" → "tofo").
            if s.flags & F_LITERAL != 0 && self.mode.classify[s.base as usize] & IS_TONE_KEY != 0 {
                return None;
            }
            if s.base == b && s.flags & F_LITERAL == 0 && s.flags & F_HORN == 0 {
                return Some(i);
            }
            if s.base == b'd' && s.flags & F_HORN != 0 {
                return None;
            }
        }
        None
    }
}
