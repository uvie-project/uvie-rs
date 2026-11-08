//! Core composing logic: keystroke processing, vowel handling, and rendering.

use crate::engine::UltraFastViEngine;
use crate::modes::{IS_MODIFIER, IS_TONE_KEY, IS_VOWEL};
use crate::modifier::{DoubleVowelLookup, ModifierHandler};
use crate::syllable::{F_CAPS, F_CIRCUMFLEX, F_HORN, F_LITERAL, F_TONE_SET, Syl};
use crate::tone_handler::ToneHandler;
use crate::validation::SyllableValidator;

/// Core composing engine: classify and handle keystrokes, render output.
pub(crate) trait Composable {
    fn process_key(&mut self, b: u8, caps: bool);
    fn apply_pending_tone(&mut self, tone_val: u8);
    fn handle_consonant(&mut self, b: u8, caps: bool);
    fn handle_vowel(&mut self, b: u8, caps: bool);
    fn promote_uo_horn_rhyme(&mut self);
    fn push_raw_key(&mut self, b: u8, caps: bool);
    fn render_out_buf(&mut self);
    fn render_passthrough(&mut self);
}

impl UltraFastViEngine {
    /// Support tones typed between the two vowels of an incomplete circumflex
    /// nucleus. In Telex the circumflex is produced by doubling the vowel
    /// (aa→â, ee→ê, oo→ô), and users often type the tone key between the two
    /// halves: `aja`→ậ, `ojo`→ộ, `ieje`→iệ, `yefe`→yề, `ueje`→uệ.
    ///
    /// This handles three patterns:
    /// - `X-e-tone-e`  → iê/yê/uê with mid tone (existing)
    /// - `a-tone-a`    → â with mid tone (new)
    /// - `o-tone-o`    → ô with mid tone (new)
    #[inline]
    fn apply_mid_nucleus_tone(&mut self, b: u8) {
        let rl = self.raw_len;
        if rl < 4 {
            return;
        }
        let first = self.raw[rl - 4];
        let mid = self.raw[rl - 3];
        let tone_key = self.raw[rl - 2];
        let last = self.raw[rl - 1];

        // Determine which double-vowel pattern this matches.
        // - iê/yê/uê: mid == 'e', last == 'e', first in {i,y,u}
        // - ê (bare): mid == 'e', last == 'e', first == 'e' (no onset)
        //   NOTE: we do NOT allow consonant onsets for the ê pattern because
        //   it conflicts with English words like "reset", "telex" where the
        //   consonant between two e's is not a tone key.
        // - â:        mid == 'a', last == 'a' (any onset)
        // - ô:        mid == 'o', last == 'o' (any onset)
        let is_ie_pattern = mid == b'e' && last == b'e' && matches!(first, b'i' | b'y' | b'u');
        let is_ee_pattern = mid == b'e' && last == b'e' && first == b'e';
        let is_aa_pattern = mid == b'a' && last == b'a';
        let is_oo_pattern = mid == b'o' && last == b'o';

        if !is_ie_pattern && !is_ee_pattern && !is_aa_pattern && !is_oo_pattern {
            return;
        }

        // The triggering keystroke must be the doubled vowel itself.
        if b != mid {
            return;
        }

        // The key between the two vowels must actually be a tone key.
        // Without this, consonants like 'l' in "telex" (t-e-l-e) would be
        // misidentified as tone keys and consumed.
        if self.mode.classify[tone_key as usize] & IS_TONE_KEY == 0 {
            return;
        }

        let tone_val = self.mode.tone[tone_key as usize];
        if tone_val == 0 {
            // Tone cancel key (e.g. Telex 'z', tone_val == 0): do nothing.
            // With the handle_tone_key fix, 'z' is pushed as a consonant when
            // no tone is set, so it stays in the buffer and the syllable
            // becomes invalid → passthrough (e.g. "thaza" → "thaza").
            // If a tone WAS set, handle_tone_key already cancelled it and 'z'
            // is not in the buffer, so there's nothing to do here either.
            return;
        }

        // The literal tone key should be the last buffer entry because the
        // second vowel modified the previous vowel in place (aa→â, etc.).
        let Some(tone_syl) = self.buf.pop() else {
            return;
        };
        if tone_syl.base != tone_key || tone_syl.flags & F_TONE_SET != 0 {
            self.buf.push(tone_syl);
            return;
        }
        if let Some(carrier) = self.tone_carrier_idx() {
            let s = self.buf.get_mut(carrier);
            s.tone = tone_val;
            s.flags |= F_TONE_SET;
            s.recompute_out();
        } else {
            self.buf.push(tone_syl);
        }
    }

    /// After a double-vowel circumflex is applied (e.g. `ie + c + s + e` →
    /// `iê + c + s`), a tone key that was pushed as a literal consonant
    /// (because the nucleus was in an intermediate state like `ie` without
    /// circumflex) may still be in the buffer. This function detects and
    /// cleans up such literal tone keys by applying their tone to the carrier.
    ///
    /// This handles patterns like:
    /// - `biecse` → `biếc` (b + i + ê + sắc + c)
    /// - `chiejne` → `chiện` (ch + i + ê + nặng + n)
    /// - `buoifo` → `buồi` (b + u + ô + huyền + i)
    /// - `biefne` → `biền` (b + i + ê + huyền + n) — tone key is NOT last entry
    /// - `diefue` → `điều` (đ + i + ê + huyền + u) — tone key between ê and u
    ///
    /// Searches from `target_idx + 1` to the end for a literal tone key.
    /// Does NOT stop at vowels, since the tone key may be between the
    /// circumflexed vowel and another nucleus vowel (e.g. `u` in `iêu`).
    #[inline]
    fn cleanup_literal_tone_after_circumflex(&mut self, b: u8, target_idx: usize) {
        // Only relevant for double-vowel circumflex keys.
        if !matches!(b, b'a' | b'e' | b'o') {
            return;
        }

        // The last raw key must be the doubled vowel.
        if self.raw_len == 0 || self.raw[self.raw_len - 1] != b {
            return;
        }

        // Search from target_idx + 1 to the end for a literal tone key.
        let n = self.buf.len();
        let mut tone_idx: Option<usize> = None;
        for i in (target_idx + 1)..n {
            let syl = self.buf.get(i);
            if syl.flags & F_TONE_SET == 0
                && self.mode.classify[syl.base as usize] & IS_TONE_KEY != 0
            {
                tone_idx = Some(i);
                break;
            }
        }

        let Some(idx) = tone_idx else {
            return;
        };

        let tone_base = self.buf.get(idx).base;
        let tone_val = self.mode.tone[tone_base as usize];
        if tone_val == 0 {
            return; // Tone cancel key (z/0) — nothing to apply.
        }

        // Remove the tone key from the buffer by shifting subsequent entries.
        self.buf.remove(idx);

        if let Some(carrier) = self.tone_carrier_idx() {
            let s = self.buf.get_mut(carrier);
            s.tone = tone_val;
            s.flags |= F_TONE_SET;
            s.recompute_out();
        }
    }

    /// Clean up literal tone keys stuck in the coda region of the buffer.
    ///
    /// When a modifier (like `w`) transforms an intermediate nucleus into a
    /// valid one (e.g. `uoi` → `ươi`), tone keys that were previously pushed
    /// as literal consonants (because the nucleus was invalid) are now stuck
    /// in the coda. This function finds and removes them, applying their tone
    /// to the tone carrier.
    ///
    /// Example: `buoirw` → after `w` makes `ươi`, the literal `r` (hỏi) in
    /// the coda is removed and applied to `ơ` → `bưởi`.
    ///
    /// Returns `true` if a tone key was found and removed.
    #[inline]
    pub(crate) fn cleanup_coda_tone_keys(&mut self) -> bool {
        let (_, _, _, coda_start) = self.partition_syllable();
        let n = self.buf.len();
        if coda_start >= n {
            return false;
        }

        // Search the coda for a literal tone key.
        let mut tone_idx: Option<usize> = None;
        for i in coda_start..n {
            let syl = self.buf.get(i);
            if syl.flags & F_TONE_SET == 0
                && self.mode.classify[syl.base as usize] & IS_TONE_KEY != 0
            {
                tone_idx = Some(i);
                break;
            }
        }

        let Some(idx) = tone_idx else {
            return false;
        };

        let tone_base = self.buf.get(idx).base;
        let tone_val = self.mode.tone[tone_base as usize];
        if tone_val == 0 {
            return false; // Tone cancel key — nothing to apply.
        }

        self.buf.remove(idx);

        if let Some(carrier) = self.tone_carrier_idx() {
            let s = self.buf.get_mut(carrier);
            s.tone = tone_val;
            s.flags |= F_TONE_SET;
            s.recompute_out();
        }
        true
    }
}

impl Composable for UltraFastViEngine {
    #[inline]
    fn process_key(&mut self, b: u8, caps: bool) {
        let attr = self.mode.classify[b as usize];

        // A tone deferred right after `dd`/`d9` (see `handle_tone_key`) only
        // survives until the next keystroke: a vowel adopts it, anything
        // else drops it — matching the old silent swallow for those cases.
        let pending_tone = self.pending_tone.take();

        if attr & IS_TONE_KEY != 0 {
            self.handle_tone_key(b, caps);
        } else if attr & IS_MODIFIER != 0 {
            self.handle_modifier(b, caps);
        } else if attr & IS_VOWEL != 0 {
            self.handle_vowel(b, caps);
            if let Some((_, _, tone_val)) = pending_tone {
                self.apply_pending_tone(tone_val);
            }
        } else {
            self.handle_consonant(b, caps);
        }
    }

    /// Apply a deferred post-`dd` tone value onto the current tone carrier.
    /// No-op when the vowel failed to create a nucleus.
    #[inline]
    fn apply_pending_tone(&mut self, tone_val: u8) {
        if let Some(carrier) = self.tone_carrier_idx() {
            let s = self.buf.get_mut(carrier);
            s.tone = tone_val;
            s.flags |= F_TONE_SET;
            s.recompute_out();
        }
    }

    #[inline]
    fn handle_consonant(&mut self, b: u8, caps: bool) {
        // Deferred /uə/ promotion: [u,ơ] (from u + w) becomes [ư,ơ] once a
        // coda follows — the closed /uə/ rhyme with a horn is written "ươ"
        // ("thuowcs" → "thước", "thuowng" → "thương", "nguowcj" → "ngược").
        self.promote_uo_horn_rhyme();
        self.buf.push(Syl::literal(b, caps));
    }

    #[inline]
    fn handle_vowel(&mut self, b: u8, caps: bool) {
        // Check for double-vowel modifier (aa→â, ee→ê, oo→ô).
        if matches!(b, b'a' | b'e' | b'o') {
            if let Some(target_idx) = self.find_modifier_target_for_double_vowel(b) {
                let syl = self.buf.get(target_idx).clone();
                // Triple-cancel: if target already has circumflex, revert to literal.
                if syl.flags & F_CIRCUMFLEX != 0 {
                    // A toned circumflex vowel is a resolved rhyme (ố, ồ, ậ —
                    // e.g. the /uə/ resolution "thuocs" → "thuốc"), not a
                    // user-typed "aa" awaiting cancel. Absorb the repeated
                    // vowel instead of destroying the rhyme ("thuocso" stays
                    // "thuốc").
                    if syl.flags & F_TONE_SET != 0 && self.is_valid_vietnamese() {
                        return;
                    }
                    if self.is_valid_vietnamese() {
                        let reverted = Syl::literal(syl.base, syl.flags & F_CAPS != 0);
                        self.buf.set(target_idx, reverted);
                        self.buf.push(Syl::literal(b, caps));
                        if self.raw_len > 0 {
                            self.raw_len -= 1;
                        }
                        self.mark_all_literal();
                        return;
                    }
                } else {
                    let snap = self.buf.get(target_idx).clone();
                    let updated = syl.with_circumflex();
                    self.buf.set(target_idx, updated);
                    self.reapply_tone_after_nucleus_change();
                    self.apply_mid_nucleus_tone(b);
                    self.cleanup_literal_tone_after_circumflex(b, target_idx);
                    // Check if the result is valid. If not, revert and push
                    // the vowel as a literal (e.g. "khoafo" where pressing 'o'
                    // after "khòa" would form invalid "ôà" — instead keep "oao").
                    if self.is_valid_vietnamese() {
                        return;
                    }
                    // Revert: undo circumflex and any tone changes.
                    self.buf.set(target_idx, snap);
                    self.reapply_tone_after_nucleus_change();
                    // Fall through to push as literal.
                }
            }
        }

        // Auto-promote: typing 'o' right after 'ư' (u with horn) should form
        // the "ươ" diphthong, mirroring the "uo" + w → "ươ" shortcut but for
        // the reverse typing order (uw first, then o). Without this,
        // "nguwocs" produces "ngứoc" instead of "ngước".
        if b == b'o' && !self.buf.is_empty() {
            let prev = self.buf.get(self.buf.len() - 1);
            if prev.base == b'u' && prev.flags & F_HORN != 0 && prev.flags & F_CIRCUMFLEX == 0 {
                let promoted = Syl::literal(b'o', caps).with_horn();
                self.buf.push(promoted);
                self.reapply_tone_after_nucleus_change();
                return;
            }
        }

        // Deferred /uə/ promotion: [u,ơ] + i/u completes the triphthong
        // ("huowi" → "ươi", "huowu" → "ươu") — promote the u to ư first.
        if matches!(b, b'i' | b'u') && !self.buf.is_empty() {
            self.promote_uo_horn_rhyme();
        }

        // Plain vowel - just push.
        self.buf.push(Syl::literal(b, caps));
        self.reapply_tone_after_nucleus_change();
    }

    /// Promote the deferred /uə/ horn rhyme: nucleus exactly [u plain, ơ
    /// (horned o)] → horn the u ("thuở" + coda → "thước"-family, "huowi" →
    /// "ươi"). No-op in every other configuration.
    #[inline]
    fn promote_uo_horn_rhyme(&mut self) {
        let n = self.buf.len();
        if n < 2 {
            return;
        }
        let (_, nucleus_start, nucleus_end, coda_start) = self.partition_syllable();
        // Only an open [u,ơ] nucleus (no coda yet — the caller promotes before
        // pushing the new entry) with exactly two vowels.
        if coda_start < n || nucleus_end - nucleus_start != 2 {
            return;
        }
        let u_syl = self.buf.get(nucleus_start);
        let o_syl = self.buf.get(nucleus_start + 1);
        if u_syl.base != b'u' || o_syl.base != b'o' {
            return;
        }
        if u_syl.flags & (F_HORN | F_CIRCUMFLEX) != 0 || o_syl.flags & F_HORN == 0 {
            return;
        }
        let promoted = self.buf.get(nucleus_start).clone().with_horn();
        self.buf.set(nucleus_start, promoted);
        self.reapply_tone_after_nucleus_change();
    }

    #[inline]
    fn push_raw_key(&mut self, b: u8, caps: bool) {
        if self.raw_len >= 24 {
            return;
        }
        // A deferred post-`dd` tone key was removed from `raw` while
        // pending; restore it just before the vowel that adopts it so the
        // raw stream (and every re-render from it) keeps the tone key.
        if let Some((pb, pcaps, _)) = self.pending_tone {
            if self.mode.classify[b as usize] & IS_VOWEL != 0 && self.raw_len < 24 {
                self.raw[self.raw_len] = pb;
                self.raw_caps[self.raw_len] = pcaps;
                self.raw_len += 1;
            }
        }
        if self.raw_len >= 24 {
            return;
        }
        self.raw[self.raw_len] = b;
        self.raw_caps[self.raw_len] = caps;
        self.raw_len += 1;
        self.process_key(b, caps);
    }

    #[inline]
    fn render_out_buf(&mut self) {
        self.update_syl_structure();

        // In traditional orthography, tone for oa/oe/uy goes on the first
        // vowel — but only for open syllables. With any coda, the tone must
        // be on the second vowel: "đoán" not "đóan", "hoạt" not "họat".
        // Applied at render time because the tone is placed before the coda
        // is typed.
        self.apply_coda_tone_rule();

        self.out_buf.clear();
        let n = self.buf.len();
        if n == 0 {
            return;
        }

        // Check validity first. If the word is valid Vietnamese, render it
        // normally even if some entries have F_LITERAL (e.g. after triple-
        // cancel of circumflex, the user may continue typing to form a valid
        // word like "booongs" → "boóng"). F_LITERAL only prevents further
        // modifier transforms, not rendering.
        if !self.is_valid_vietnamese() {
            self.render_passthrough();
            return;
        }

        // Valid Vietnamese - render resolved chars from buf.
        for i in 0..n {
            let s = self.buf.get(i);
            let c = s.render();
            let _ = self.out_buf.push(c);
        }
    }

    #[inline]
    fn render_passthrough(&mut self) {
        let n_buf = self.buf.len();
        let mut buf_idx = 0usize;
        let mut raw_idx = 0usize;
        while raw_idx < self.raw_len {
            let b = self.raw[raw_idx];
            let is_dh = buf_idx < n_buf
                && self.buf.get(buf_idx).base == b'd'
                && self.buf.get(buf_idx).flags & F_LITERAL == 0
                && self.buf.get(buf_idx).flags & crate::syllable::F_HORN != 0;
            if is_dh && b == b'd' && raw_idx + 1 < self.raw_len && self.raw[raw_idx + 1] == b'd' {
                let is_upper = self.buf.get(buf_idx).flags & crate::syllable::F_CAPS != 0;
                let _ = self.out_buf.push(if is_upper { 'Đ' } else { 'đ' });
                raw_idx += 2;
                buf_idx += 1;
            } else {
                let c = if self.raw_caps[raw_idx] {
                    (b as char).to_ascii_uppercase()
                } else {
                    b as char
                };
                let _ = self.out_buf.push(c);
                raw_idx += 1;
                buf_idx += 1;
            }
        }
    }
}
