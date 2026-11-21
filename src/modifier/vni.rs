//! VNI-specific modifier keys: `6` (circumflex), `7` (horn), `8` (breve), `9` (đ).
//!
//! Modifier digits target the NUCLEUS of the current syllable (found via
//! `partition_syllable`), so they keep working after a coda consonant was
//! typed ("bien6" -> "biên", "tieng61" -> "tiếng"). Candidates are tried
//! from last to first and the first result that is a valid Vietnamese
//! syllable wins; if none validates but the last buffer entry is itself a
//! candidate vowel (adjacent typing, mid-growth), it is applied anyway,
//! and a digit with no target stays a literal digit.

use crate::engine::UltraFastViEngine;
use crate::modes::IS_VOWEL;
use crate::syllable::{F_CAPS, F_CIRCUMFLEX, F_HORN, F_LITERAL, Syl};
use crate::tone_handler::ToneHandler;
use crate::validation::SyllableValidator;

/// Modifier-key cancel: the doubled digit undoes the transform. Fires only
/// when the transform was applied by the SAME digit (a true double-press):
/// the applying key leaves `raw` (it was consumed) and the cancel key stays,
/// so the literal digit renders - "a66" -> "a6", "d99" -> "d9". Returns false
/// when no earlier same digit exists ("dd9" -> "d", "aa6" -> "a"): the key
/// then just reverts the transform and vanishes as before.
fn push_cancel_digit(e: &mut UltraFastViEngine, digit: u8, caps: bool) -> bool {
    for i in (0..e.raw_len.saturating_sub(1)).rev() {
        if e.raw[i] == digit {
            e.raw.copy_within(i + 1..e.raw_len, i);
            e.raw_caps.copy_within(i + 1..e.raw_len, i);
            e.raw_len -= 1;
            e.buf.push(Syl::literal(digit, caps));
            e.diff.word_had_modifier_cancel = true;
            return true;
        }
    }
    false
}

/// Whether the key before this `7` was another `7`. The consumed redundant 7
/// is dropped from `raw` but stays in `diff.raw_chars`, so check both.
fn prev_key_is_7(e: &UltraFastViEngine) -> bool {
    let nchars = e.diff.raw_chars.len();
    (e.raw_len >= 2 && e.raw[e.raw_len - 2] == b'7')
        || (nchars >= 2 && e.diff.raw_chars[nchars - 2] == '7')
}

/// VNI-mode modifier handling (`6` → circumflex, `7` → horn, `8` → breve, `9` → đ).
pub(crate) trait VniModifier {
    fn handle_vni_6(&mut self, caps: bool);
    fn handle_vni_7(&mut self, caps: bool);
    fn handle_vni_8(&mut self, caps: bool);
    fn handle_vni_9(&mut self, caps: bool);
}

impl VniModifier for UltraFastViEngine {
    #[inline]
    fn handle_vni_6(&mut self, caps: bool) {
        let (_, nucleus_start, nucleus_end, _) = self.partition_syllable();
        let mut candidates = [0usize; 24];
        let mut candidate_count = 0usize;
        for i in (nucleus_start..nucleus_end).rev() {
            let syl = self.buf.get(i);
            if matches!(syl.base, b'a' | b'e' | b'o') && syl.flags & F_LITERAL == 0 {
                candidates[candidate_count] = i;
                candidate_count += 1;
            }
        }

        for k in 0..candidate_count {
            let i = candidates[k];
            let syl = *self.buf.get(i);
            if syl.flags & F_CIRCUMFLEX != 0 {
                // Cancel: revert to the plain vowel, keep the second 6 literal.
                let reverted = Syl::literal(syl.base, syl.flags & F_CAPS != 0);
                self.buf.set(i, reverted);
                push_cancel_digit(self, b'6', caps);
                self.reapply_tone_after_nucleus_change();
                return;
            }
            let snap_i = *self.buf.get(i);
            let snap_prev = if i > nucleus_start {
                Some(*self.buf.get(i - 1))
            } else {
                None
            };
            let snap_len = self.buf.len();
            // Apply the circumflex, swapping away any horn/breve. For a
            // resolved [ư,ơ] rhyme the u un-horns too: "ươ" + 6 -> "uô".
            let updated = self.buf.get(i).clone().with_circumflex();
            self.buf.set(i, updated);
            if syl.base == b'o' && i > nucleus_start {
                let prev = self.buf.get(i - 1);
                if prev.base == b'u' && prev.flags & F_HORN != 0 {
                    let mut unhorned = *prev;
                    unhorned.flags &= !F_HORN;
                    unhorned.recompute_out();
                    self.buf.set(i - 1, unhorned);
                }
            }
            self.reapply_tone_after_nucleus_change();
            if self.is_valid_vietnamese() {
                return;
            }
            self.buf.set(i, snap_i);
            if let Some(prev) = snap_prev {
                self.buf.set(i - 1, prev);
            }
            while self.buf.len() > snap_len {
                self.buf.pop();
            }
        }

        // Adjacent fallback: no candidate validated, but the last entry is a
        // candidate vowel - apply anyway so mid-growth typing keeps working.
        let n = self.buf.len();
        if n > 0 {
            let syl = *self.buf.get(n - 1);
            if matches!(syl.base, b'a' | b'e' | b'o') && syl.flags & F_LITERAL == 0 {
                if syl.flags & F_CIRCUMFLEX != 0 {
                    let reverted = Syl::literal(syl.base, syl.flags & F_CAPS != 0);
                    self.buf.set(n - 1, reverted);
                    push_cancel_digit(self, b'6', caps);
                } else {
                    let updated = syl.with_circumflex();
                    self.buf.set(n - 1, updated);
                }
                self.reapply_tone_after_nucleus_change();
                return;
            }
        }
        self.buf.push(Syl::literal(b'6', false));
    }

    #[inline]
    fn handle_vni_7(&mut self, caps: bool) {
        let n = self.buf.len();
        let (_, nucleus_start, nucleus_end, coda_start) = self.partition_syllable();
        let prev_is_7 = prev_key_is_7(self);

        // An adjacent [u,o] nucleus pair is the /uə/ rhyme: horn follows the
        // Telex `w` semantics - both vowels when a coda or a trailing vowel
        // closes it ("uoi", "uou"), only the o on an open rhyme ("uơ").
        for i in (nucleus_start..nucleus_end.saturating_sub(1)).rev() {
            let u = *self.buf.get(i);
            let o = *self.buf.get(i + 1);
            if u.base != b'u' || o.base != b'o' || self.is_u_glide(i) {
                continue;
            }
            let u_horned = u.flags & F_HORN != 0;
            let o_horned = o.flags & F_HORN != 0;
            if u_horned && o_horned {
                if prev_is_7 {
                    // Double-7 cancel: revert both vowels, keep the last 7
                    // literal ("u7o77" -> "uo7"), same as the Telex 'o'-cancel
                    // un-horning the u before it.
                    let reverted = Syl::literal(o.base, o.flags & F_CAPS != 0);
                    self.buf.set(i + 1, reverted);
                    let mut unhorned = u;
                    unhorned.flags &= !F_HORN;
                    unhorned.recompute_out();
                    self.buf.set(i, unhorned);
                    push_cancel_digit(self, b'7', caps);
                    self.reapply_tone_after_nucleus_change();
                } else {
                    // The o was auto-horned when typed after ư: the key is
                    // redundant, consume it silently like a redundant w.
                    if self.raw_len > 0 {
                        self.raw_len -= 1;
                    }
                }
                return;
            }
            let snap_u = u;
            let snap_o = o;
            let snap_len = self.buf.len();
            if !u_horned && !o_horned {
                if u.flags & F_CIRCUMFLEX != 0 || o.flags & F_CIRCUMFLEX != 0 {
                    continue;
                }
                // Horn the o; horn the u too when the rhyme is closed by a
                // coda or a following nucleus vowel.
                let updated_o = o.with_horn();
                self.buf.set(i + 1, updated_o);
                if coda_start < n || i + 2 < nucleus_end {
                    let updated_u = u.with_horn();
                    self.buf.set(i, updated_u);
                }
            } else if u_horned && !o_horned {
                let updated_o = o.with_horn();
                self.buf.set(i + 1, updated_o);
            } else {
                // [u,ơ]: only the u can still change - handled by the single
                // candidate pass below.
                continue;
            }
            self.reapply_tone_after_nucleus_change();
            if self.is_valid_vietnamese() {
                return;
            }
            self.buf.set(i, snap_u);
            self.buf.set(i + 1, snap_o);
            while self.buf.len() > snap_len {
                self.buf.pop();
            }
        }

        // Single candidates: o first (last to first), then u (first to first,
        // skipping the second u of a "uu" pair and the "qu" glide).
        let mut singles = [0usize; 24];
        let mut single_count = 0usize;
        for i in (nucleus_start..nucleus_end).rev() {
            let syl = self.buf.get(i);
            if syl.base == b'o' && syl.flags & F_LITERAL == 0 {
                singles[single_count] = i;
                single_count += 1;
            }
        }
        for i in nucleus_start..nucleus_end {
            let syl = self.buf.get(i);
            if syl.base == b'u' && syl.flags & F_LITERAL == 0 && !self.is_u_glide(i) {
                if i > nucleus_start && self.buf.get(i - 1).base == b'u' {
                    continue;
                }
                singles[single_count] = i;
                single_count += 1;
            }
        }
        for k in 0..single_count {
            let i = singles[k];
            let syl = *self.buf.get(i);
            if syl.flags & F_HORN != 0 {
                // Cancel: revert to the plain vowel, keep the second 7
                // literal. A horned u right before a horned o un-horns too
                // (Telex 'o'-cancel parity).
                let reverted = Syl::literal(syl.base, syl.flags & F_CAPS != 0);
                self.buf.set(i, reverted);
                if syl.base == b'o' && i > nucleus_start {
                    let prev = self.buf.get(i - 1);
                    if prev.base == b'u' && prev.flags & F_HORN != 0 {
                        let mut unhorned = *prev;
                        unhorned.flags &= !F_HORN;
                        unhorned.recompute_out();
                        self.buf.set(i - 1, unhorned);
                    }
                }
                push_cancel_digit(self, b'7', caps);
                self.reapply_tone_after_nucleus_change();
                return;
            }
            let snap_i = *self.buf.get(i);
            let snap_len = self.buf.len();
            let updated = syl.with_horn();
            self.buf.set(i, updated);
            self.reapply_tone_after_nucleus_change();
            if self.is_valid_vietnamese() {
                return;
            }
            self.buf.set(i, snap_i);
            while self.buf.len() > snap_len {
                self.buf.pop();
            }
        }

        // Adjacent fallback: apply to the last entry when it is a candidate.
        let n = self.buf.len();
        if n > 0 {
            let syl = *self.buf.get(n - 1);
            if matches!(syl.base, b'o' | b'u') && syl.flags & F_LITERAL == 0 {
                if syl.flags & F_HORN != 0 {
                    let reverted = Syl::literal(syl.base, syl.flags & F_CAPS != 0);
                    self.buf.set(n - 1, reverted);
                    push_cancel_digit(self, b'7', caps);
                } else {
                    let updated = syl.with_horn();
                    self.buf.set(n - 1, updated);
                }
                self.reapply_tone_after_nucleus_change();
                return;
            }
        }
        self.buf.push(Syl::literal(b'7', false));
    }

    #[inline]
    fn handle_vni_8(&mut self, caps: bool) {
        let (_, nucleus_start, nucleus_end, _) = self.partition_syllable();
        let mut candidates = [0usize; 24];
        let mut candidate_count = 0usize;
        for i in (nucleus_start..nucleus_end).rev() {
            let syl = self.buf.get(i);
            if syl.base == b'a' && syl.flags & F_LITERAL == 0 {
                candidates[candidate_count] = i;
                candidate_count += 1;
            }
        }

        for k in 0..candidate_count {
            let i = candidates[k];
            let syl = *self.buf.get(i);
            if syl.flags & F_HORN != 0 {
                // Cancel: revert to the plain vowel, keep the second 8 literal.
                let reverted = Syl::literal(syl.base, syl.flags & F_CAPS != 0);
                self.buf.set(i, reverted);
                push_cancel_digit(self, b'8', caps);
                self.reapply_tone_after_nucleus_change();
                return;
            }
            let snap_i = *self.buf.get(i);
            let snap_len = self.buf.len();
            let updated = syl.with_horn();
            self.buf.set(i, updated);
            self.reapply_tone_after_nucleus_change();
            if self.is_valid_vietnamese() {
                return;
            }
            self.buf.set(i, snap_i);
            while self.buf.len() > snap_len {
                self.buf.pop();
            }
        }

        // Adjacent fallback.
        let n = self.buf.len();
        if n > 0 {
            let syl = *self.buf.get(n - 1);
            if syl.base == b'a' && syl.flags & F_LITERAL == 0 {
                if syl.flags & F_HORN != 0 {
                    let reverted = Syl::literal(syl.base, syl.flags & F_CAPS != 0);
                    self.buf.set(n - 1, reverted);
                    push_cancel_digit(self, b'8', caps);
                } else {
                    let updated = syl.with_horn();
                    self.buf.set(n - 1, updated);
                }
                self.reapply_tone_after_nucleus_change();
                return;
            }
        }
        self.buf.push(Syl::literal(b'8', false));
    }

    #[inline]
    fn handle_vni_9(&mut self, caps: bool) {
        // The target is a `d` in the onset - everything before it is a
        // consonant - reachable from anywhere in the word ("duoc9" -> "đuoc").
        for i in 0..self.buf.len() {
            let syl = self.buf.get(i);
            if self.mode.classify[syl.base as usize] & IS_VOWEL != 0 || syl.base == b'w' {
                break;
            }
            if syl.base == b'd' && syl.flags & F_LITERAL == 0 {
                if syl.flags & F_HORN != 0 {
                    let reverted = Syl::literal(syl.base, syl.flags & F_CAPS != 0);
                    self.buf.set(i, reverted);
                    push_cancel_digit(self, b'9', caps);
                    self.mark_all_literal();
                } else {
                    let new_syl = Syl {
                        base: b'd',
                        out: if syl.flags & F_CAPS != 0 { 'Đ' } else { 'đ' },
                        tone: 0,
                        flags: syl.flags | F_HORN,
                    };
                    self.buf.set(i, new_syl);
                }
                return;
            }
        }
        self.buf.push(Syl::literal(b'9', false));
    }
}
