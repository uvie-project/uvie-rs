//! Telex-specific modifier keys: `w` (horn/ư) and `d` (đ).

use crate::engine::UltraFastViEngine;
use crate::modes::IS_VOWEL;
use crate::syllable::{F_CAPS, F_CIRCUMFLEX, F_HORN, F_LITERAL, Syl};
use crate::tone_handler::ToneHandler;
use crate::validation::SyllableValidator;

/// Telex-mode modifier handling (`w` → horn/ư, `d` → đ).
pub(crate) trait TelexModifier {
    fn handle_telex_w(&mut self, caps: bool);
    fn handle_telex_d(&mut self, caps: bool);
    fn try_apply_w_non_cancel(&mut self, idx: usize, nucleus_start: usize, caps: bool) -> bool;
}

impl TelexModifier for UltraFastViEngine {
    #[inline]
    fn handle_telex_w(&mut self, caps: bool) {
        let n = self.buf.len();

        // Find nucleus boundaries so w can modify vowels even with coda present
        let (_onset_end, nucleus_start, nucleus_end, _coda_start) = self.partition_syllable();

        // Collect w-target candidates (u, o, a) in the nucleus, in backwards order.
        let mut candidates = [0usize; 24];
        let mut candidate_count = 0usize;
        for i in (nucleus_start..nucleus_end).rev() {
            let syl = self.buf.get(i);
            if matches!(syl.base, b'u' | b'o' | b'a') {
                candidates[candidate_count] = i;
                candidate_count += 1;
            }
        }

        // First pass: try each candidate. If a target produces a valid Vietnamese
        // syllable, keep it. This fixes cases like "chuaw" -> "chưa" where the
        // backwards-first heuristic would otherwise modify the wrong vowel.
        //
        // Optimization: use single-entry snapshot for the fast path (common
        // case where no coda cleanup is needed). Only fall back to full clone
        // when coda cleanup is required (rare, e.g. "buoirw").
        for idx in 0..candidate_count {
            let i = candidates[idx];
            // Fast path: single-entry snapshot (target + neighbor for promotion).
            let snap_i = *self.buf.get(i);
            let snap_neighbor = if i > 0 {
                Some(*self.buf.get(i - 1))
            } else {
                None
            };
            let snap_len = self.buf.len();
            if self.try_apply_w_non_cancel(i, nucleus_start, caps) {
                if self.is_valid_vietnamese() {
                    return;
                }
                // Check if coda has a literal tone key that could be cleaned up.
                let (_, _, _, coda_start) = self.partition_syllable();
                let has_coda_tone = (coda_start..self.buf.len()).any(|j| {
                    let s = self.buf.get(j);
                    s.flags & crate::syllable::F_TONE_SET == 0
                        && s.flags & crate::syllable::F_LITERAL == 0
                        && self.mode.classify[s.base as usize] & crate::modes::IS_TONE_KEY != 0
                });
                if has_coda_tone {
                    // Slow path: full clone needed for coda cleanup (removes entry).
                    let snap_buf = self.buf.clone();
                    self.cleanup_coda_tone_keys();
                    if self.is_valid_vietnamese() {
                        return;
                    }
                    self.buf = snap_buf;
                }
                // Restore single-entry snapshot.
                self.buf.set(i, snap_i);
                if let Some(n) = snap_neighbor {
                    self.buf.set(i - 1, n);
                }
                while self.buf.len() > snap_len {
                    self.buf.pop();
                }
            } else {
                // try_apply_w_non_cancel returned false — no changes made.
            }
        }

        // No valid candidate found. If there is only a single candidate and it
        // can be applied (i.e. it does not already carry F_HORN), preserve the
        // original behaviour by applying it even if the result is invalid (so
        // double-w cancellation works for e.g. "showw" -> "show").
        if candidate_count == 1 {
            let i = candidates[0];
            let snap_i = *self.buf.get(i);
            let snap_len = self.buf.len();
            if self.try_apply_w_non_cancel(i, nucleus_start, caps) {
                return;
            }
            self.buf.set(i, snap_i);
            while self.buf.len() > snap_len {
                self.buf.pop();
            }
        }

        // ---- Simple Telex mode: vneHookAll ----
        // In Simple Telex, `w` only applies horn/breve to vowel candidates.
        // No standalone `w→ư`, no double-w cancel, no silent consume.
        // If no candidate worked, push `w` as a literal consonant.
        if self.is_simple_telex {
            self.buf.push(Syl::literal(b'w', caps));
            return;
        }

        // ---- Regular Telex: cancellation, silent consume, standalone ư ----
        // If ALL candidates already have F_HORN and the syllable is valid,
        // the `w` is redundant (e.g. "chuwongw" where both u→ư and o→ơ
        // already have horn). Consume it silently instead of cancelling.
        // Only apply when there are multiple candidates — for a single
        // candidate, the cancellation pass below should handle it (e.g.
        // "aww" → "aw" needs cancellation, not silent consume).
        // Also skip when the previous raw key was also 'w' (double-w
        // cancellation like "uoww" → "uow" should be handled by the
        // cancellation pass, not consumed silently).
        let prev_is_w = self.raw_len >= 2 && self.raw[self.raw_len - 2] == b'w';
        if candidate_count > 1 && !prev_is_w {
            let all_horned =
                (0..candidate_count).all(|idx| self.buf.get(candidates[idx]).flags & F_HORN != 0);
            if all_horned && self.is_valid_vietnamese() {
                if self.raw_len > 0 {
                    self.raw_len -= 1;
                }
                return;
            }
        }

        // If no candidate produced a valid syllable but the current syllable
        // is already valid, consume `w` silently (e.g. "buwouw" where `w`
        // after `bươu` would make `ươư` invalid — just drop the `w`).
        // Only apply when there are multiple candidates and previous key
        // was not 'w' (double-w cancellation should go through cancel pass).
        if candidate_count > 1 && !prev_is_w && self.is_valid_vietnamese() {
            if self.raw_len > 0 {
                self.raw_len -= 1;
            }
            return;
        }

        // Second pass: cancellation (target already has F_HORN, or existing 'w').
        for i in (nucleus_start..nucleus_end).rev() {
            let syl = self.buf.get(i);
            match syl.base {
                b'u' => {
                    if self.is_u_glide(i) {
                        continue;
                    }
                    if i > nucleus_start && self.buf.get(i - 1).base == b'u' {
                        continue;
                    }
                    if syl.flags & F_HORN != 0 {
                        let reverted = Syl::literal(syl.base, syl.flags & F_CAPS != 0);
                        self.buf.set(i, reverted);
                        if self.raw_len > 0 {
                            self.raw_len -= 1;
                        }
                        self.buf.push(Syl::literal(b'w', caps));
                        self.reapply_tone_after_nucleus_change();
                        self.diff.word_had_modifier_cancel = true;
                        return;
                    }
                }
                b'o' => {
                    if syl.flags & F_HORN != 0 {
                        let reverted = Syl::literal(syl.base, syl.flags & F_CAPS != 0);
                        self.buf.set(i, reverted);
                        if self.raw_len > 0 {
                            self.raw_len -= 1;
                        }
                        self.buf.push(Syl::literal(b'w', caps));
                        if i > 0 {
                            let prev = self.buf.get(i - 1);
                            if prev.base == b'u' && prev.flags & F_HORN != 0 {
                                let reverted_u = Syl::literal(b'u', prev.flags & F_CAPS != 0);
                                self.buf.set(i - 1, reverted_u);
                            }
                        }
                        self.reapply_tone_after_nucleus_change();
                        self.diff.word_had_modifier_cancel = true;
                        return;
                    }
                }
                b'a' => {
                    if syl.flags & F_CIRCUMFLEX != 0 {
                        continue;
                    }
                    if syl.flags & F_HORN != 0 {
                        let reverted = Syl::literal(syl.base, syl.flags & F_CAPS != 0);
                        self.buf.set(i, reverted);
                        if self.raw_len > 0 {
                            self.raw_len -= 1;
                        }
                        self.buf.push(Syl::literal(b'w', caps));
                        self.reapply_tone_after_nucleus_change();
                        self.diff.word_had_modifier_cancel = true;
                        return;
                    }
                }
                _ => {}
            }
        }

        // Third pass: look for existing 'w' with F_HORN (for cancellation)
        for i in (0..n).rev() {
            let syl = self.buf.get(i);
            if syl.base == b'w' && syl.flags & F_HORN != 0 {
                let reverted = Syl::literal(b'w', syl.flags & F_CAPS != 0);
                self.buf.set(i, reverted);
                if self.raw_len > 0 {
                    self.raw_len -= 1;
                }
                self.buf.push(Syl::literal(b'w', caps));
                self.reapply_tone_after_nucleus_change();
                self.diff.word_had_modifier_cancel = true;
                return;
            }
            // Stop searching when we hit consonants after checking w-cancellation
            if self.mode.classify[syl.base as usize] & IS_VOWEL == 0 && syl.base != b'w' {
                break;
            }
        }

        // No match - standalone 'w' becomes ư at onset.
        let onset_len = self.onset_len();
        // A second `w` right after a `w` that stayed a literal letter is the
        // cancel sequence for a modifier that never applied: consume it
        // ("neww" → "new", "awwa" → "awa"). Only in a syllable that carries no
        // Vietnamese modifier and no w-target at all — there a `w` is a plain
        // letter the second one undoes. A vowel that merely could not take the
        // horn keeps both keys, as it always has ("aaww", "eeww", "showw").
        let plain_syllable = (0..n).all(|i| self.buf.get(i).flags & (F_HORN | F_CIRCUMFLEX) == 0);
        let cancel_literal_w = plain_syllable && candidate_count == 0 && prev_is_w && n > 0 && {
            let prev = self.buf.get(n - 1);
            prev.base == b'w' && prev.flags & (F_HORN | F_CIRCUMFLEX | F_LITERAL) == 0
        };
        if onset_len == n {
            let mut syl = Syl::literal(b'w', caps);
            syl.flags |= F_HORN;
            syl.out = if caps { 'Ư' } else { 'ư' };
            self.buf.push(syl);
            self.reapply_tone_after_nucleus_change();
        } else if cancel_literal_w {
            if self.raw_len > 0 {
                self.raw_len -= 1;
            }
            self.diff.word_had_modifier_cancel = true;
        } else {
            self.buf.push(Syl::literal(b'w', caps));
        }
    }

    /// Apply the non-cancelling w modifier to a single nucleus target and return
    /// true if the modification was applied. Does not touch candidates that
    /// already carry F_HORN (those are handled in the cancellation pass).
    fn try_apply_w_non_cancel(&mut self, idx: usize, nucleus_start: usize, _caps: bool) -> bool {
        let syl = self.buf.get(idx);
        if syl.flags & F_HORN != 0 {
            return false;
        }
        match syl.base {
            b'u' => {
                if self.is_u_glide(idx) {
                    return false;
                }
                // Skip the second 'u' in a consecutive "uu" inside the nucleus.
                if idx > nucleus_start && self.buf.get(idx - 1).base == b'u' {
                    return false;
                }
                // [u,ơ] state (the o already horned): horn-ing the u here is
                // the deferred /ɨə/ promotion — it must not fire on a second
                // 'w' ("uoww" → "uow" is a cancel, not "ươ"). The promotion
                // happens when a coda or triphthong vowel follows instead.
                if idx + 1 < self.buf.len() {
                    let next = self.buf.get(idx + 1);
                    if next.base == b'o' && next.flags & F_HORN != 0 {
                        return false;
                    }
                }
                let updated = self.buf.get(idx).clone().with_horn();
                self.buf.set(idx, updated);
                self.reapply_tone_after_nucleus_change();
                true
            }
            b'o' => {
                // Resolved uô form (from the /uə/ tone resolution or the oo
                // merge) + w → the /ɨə/ interpretation: swap the circumflex
                // for a horn and promote a preceding plain u
                // ("thuocsw" → "thước", "xuongsw" → "xướng").
                if syl.flags & F_CIRCUMFLEX != 0 {
                    let mut swapped = Syl {
                        base: syl.base,
                        out: syl.out,
                        tone: syl.tone,
                        flags: (syl.flags & !F_CIRCUMFLEX) | F_HORN,
                    };
                    swapped.recompute_out();
                    self.buf.set(idx, swapped);
                    if idx > 0 && idx > nucleus_start {
                        let prev = self.buf.get(idx - 1);
                        if prev.base == b'u'
                            && prev.flags & (F_HORN | F_CIRCUMFLEX) == 0
                            && !self.is_u_glide(idx - 1)
                        {
                            let promoted = prev.clone().with_horn();
                            self.buf.set(idx - 1, promoted);
                        }
                    }
                    self.reapply_tone_after_nucleus_change();
                    return true;
                }
                let updated = self.buf.get(idx).clone().with_horn();
                self.buf.set(idx, updated);
                // Deferred /uə/ promotion fires immediately when the [u,ơ] is
                // not a final open syllable: a coda or a trailing nucleus
                // vowel follows ("buonw" → "bươn", "buoiw" → "ươi",
                // "thuocws" → "thước"). A bare open "huow" stays "huơ".
                let n2 = self.buf.len();
                let (_, _, nucleus_end, coda_start) = self.partition_syllable();
                if coda_start < n2 || idx + 1 < nucleus_end {
                    if idx > 0 {
                        let prev = self.buf.get(idx - 1);
                        if prev.base == b'u'
                            && prev.flags & (F_HORN | F_CIRCUMFLEX) == 0
                            && !self.is_u_glide(idx - 1)
                        {
                            let promoted = prev.clone().with_horn();
                            self.buf.set(idx - 1, promoted);
                        }
                    }
                }
                self.reapply_tone_after_nucleus_change();
                true
            }
            b'a' => {
                if syl.flags & F_CIRCUMFLEX != 0 {
                    return false;
                }
                let updated = self.buf.get(idx).clone().with_horn();
                self.buf.set(idx, updated);
                self.reapply_tone_after_nucleus_change();
                true
            }
            _ => false,
        }
    }

    #[inline]
    fn handle_telex_d(&mut self, caps: bool) {
        let n = self.buf.len();

        for i in (0..n).rev() {
            let s = self.buf.get(i);
            if s.base == b'd' && s.flags & F_HORN != 0 {
                if self.is_valid_vietnamese() {
                    let reverted = Syl::literal(b'd', s.flags & F_CAPS != 0);
                    self.buf.set(i, reverted);
                    self.buf.push(Syl::literal(b'd', caps));
                    if self.raw_len > 0 {
                        self.raw_len -= 1;
                    }
                    self.mark_all_literal();
                    self.diff.word_had_modifier_cancel = true;
                    return;
                }
                break;
            }
            if s.base == b'd' && s.flags & F_LITERAL == 0 && s.flags & F_HORN == 0 {
                let is_in_onset = (0..i).all(|j| {
                    let sj = self.buf.get(j);
                    self.mode.classify[sj.base as usize] & IS_VOWEL == 0 && sj.base != b'w'
                });
                if !is_in_onset {
                    break;
                }
                let new_syl = Syl {
                    base: b'd',
                    out: if s.flags & F_CAPS != 0 { 'Đ' } else { 'đ' },
                    tone: 0,
                    flags: s.flags | F_HORN,
                };
                self.buf.set(i, new_syl);
                return;
            }
        }

        self.buf.push(Syl::literal(b'd', caps));
    }
}
