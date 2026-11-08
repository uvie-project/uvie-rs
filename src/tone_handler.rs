//! Tone key handling and tone carrier placement logic.

use crate::engine::UltraFastViEngine;
use crate::syllable::{F_CIRCUMFLEX, F_HORN, F_LITERAL, F_TONE_SET, Syl};
use crate::tables::{nucleus_tone_target, onset_is_gi, onset_is_qu};
use crate::validation::SyllableValidator;

/// Tone key processing and carrier placement.
pub(crate) trait ToneHandler {
    fn handle_tone_key(&mut self, b: u8, caps: bool);
    fn tone_carrier_idx(&self) -> Option<usize>;
    fn reapply_tone_after_nucleus_change(&mut self);
    fn apply_coda_tone_rule(&mut self);
    fn resolve_uo_rhyme_tone(&mut self);
    fn resolve_ech_rhyme_tone(&mut self);
    fn resolve_centering_rhyme_tone(&mut self);
}

impl ToneHandler for UltraFastViEngine {
    #[inline]
    fn handle_tone_key(&mut self, b: u8, caps: bool) {
        let tone_val = self.mode.tone[b as usize];

        let n = self.buf.len();
        if n == 0 {
            self.buf.push(Syl::literal(b, caps));
            return;
        }

        // Special 'r' heuristic: after certain onset consonants, 'r' is part of
        // the onset (e.g. "tr", "br") - not a tone key. Only apply this when the
        // previous character is actually inside the onset; if it is part of the
        // nucleus or coda, 'r' must remain a tone key.
        if b == b'r' && n > 0 {
            let prev = self.buf.get(n - 1).base;
            let (_, nucleus_start, _, _) = self.partition_syllable();
            let prev_idx = n - 1;
            if prev_idx < nucleus_start
                && matches!(prev, b't' | b'p' | b'f' | b'c' | b'b' | b'd' | b'g' | b'k')
            {
                self.buf.push(Syl::literal(b'r', caps));
                return;
            }
        }

        // If the word is already invalid Vietnamese (passthrough mode), treat
        // tone keys as plain consonants.
        if !self.is_valid_vietnamese() {
            self.buf.push(Syl::consonant(b, caps));
            return;
        }

        let carrier = self.tone_carrier_idx();

        if carrier.is_none() {
            // No nucleus yet, so there is nothing to carry a tone.
            //
            // After a modified consonant (`dd` -> `đ`, VNI `d9` -> `đ`) the
            // tone key is deferred as `pending_tone`: it stays invisible
            // (removed from `raw`, like the old silent swallow) until the
            // first vowel arrives and adopts it — "ddjuowc" becomes "được"
            // and "ddsu" becomes "đú". If the word ends or a non-vowel key
            // comes first, the pending tone is dropped and "ddf" still
            // renders "đ" (matching VNI "d91" -> "đ").
            //
            // Tone keys with no modified consonant at all ("bj...", "fj...")
            // render literally, making the syllable invalid so the word
            // passes through raw.
            let (_, ns, ne, _) = self.partition_syllable();
            if ne <= ns {
                let has_modified_consonant = (0..self.buf.len()).any(|i| {
                    let s = self.buf.get(i);
                    s.base == b'd' && s.flags & F_HORN != 0
                });
                if has_modified_consonant {
                    if tone_val != 0 {
                        self.pending_tone = Some((b, caps, tone_val));
                    }
                    if self.raw_len > 0 {
                        self.raw_len -= 1;
                    }
                    return;
                }
            }
            self.buf.push(Syl::literal(b, caps));
            return;
        }

        let carrier_idx = carrier.unwrap();

        let existing = self.buf.get(carrier_idx);
        let already_has_tone = existing.flags & F_TONE_SET != 0;

        // Tone cancel key (e.g. Telex 'z', tone_val == 0): only meaningful
        // when there is an existing tone to cancel. If no tone is set yet,
        // treat the key as a plain consonant so words like "azure" work.
        if tone_val == 0 && !already_has_tone {
            self.buf.push(Syl::consonant(b, caps));
            return;
        }

        if already_has_tone && existing.tone == tone_val && existing.flags & F_LITERAL == 0 {
            // Double-same-tone-key: cancel tone.
            let reverted = {
                let s = self.buf.get(carrier_idx);
                let mut new_s = *s;
                new_s.flags &= !(F_TONE_SET);
                new_s.tone = 0;
                new_s.recompute_out();
                new_s
            };
            self.buf.set(carrier_idx, reverted);

            if self.raw_len > 0 {
                self.raw_len -= 1;
            }
            self.buf.push(Syl::consonant(b, caps));
            return;
        }

        // Override: last tone key wins.
        {
            let s = self.buf.get_mut(carrier_idx);
            s.tone = tone_val;
            s.flags |= F_TONE_SET;
            s.recompute_out();
        }
    }

    #[inline]
    fn tone_carrier_idx(&self) -> Option<usize> {
        let n = self.buf.len();

        let (_onset_end, nucleus_start, nucleus_end, _coda_start) = self.partition_syllable();

        if nucleus_start >= nucleus_end {
            return None;
        }

        let nucleus_len = nucleus_end - nucleus_start;

        let mut nuc: [char; 3] = ['\0'; 3];
        let take = nucleus_len.min(3);
        for i in 0..take {
            nuc[i] = self.buf.get(nucleus_start + i).base_no_tone();
        }
        let nuc_slice = &nuc[..take];

        let onset_raw = self.onset_raw_slice();
        let (eff_nucleus_start, eff_nuc_slice, tone_offset) = if onset_is_qu(onset_raw)
            && nucleus_start < n
            && self.buf.get(nucleus_start).base == b'u'
        {
            let eff_start = nucleus_start + 1;
            if eff_start < nucleus_end {
                let eff_len = (nucleus_end - eff_start).min(3);
                let mut enuc = ['\0'; 3];
                for i in 0..eff_len {
                    enuc[i] = self.buf.get(eff_start + i).base_no_tone();
                }
                (eff_start, enuc, 0usize)
            } else {
                (
                    nucleus_start,
                    {
                        let mut a = ['\0'; 3];
                        a[..take].copy_from_slice(nuc_slice);
                        a
                    },
                    0,
                )
            }
        } else if onset_is_gi(onset_raw)
            && nucleus_start < n
            && self.buf.get(nucleus_start).base == b'i'
        {
            let eff_start = nucleus_start + 1;
            if eff_start < nucleus_end {
                let eff_len = (nucleus_end - eff_start).min(3);
                let mut enuc = ['\0'; 3];
                for i in 0..eff_len {
                    enuc[i] = self.buf.get(eff_start + i).base_no_tone();
                }
                (eff_start, enuc, 0usize)
            } else {
                (
                    nucleus_start,
                    {
                        let mut a = ['\0'; 3];
                        a[..take].copy_from_slice(nuc_slice);
                        a
                    },
                    0,
                )
            }
        } else {
            (
                nucleus_start,
                {
                    let mut a = ['\0'; 3];
                    a[..take].copy_from_slice(nuc_slice);
                    a
                },
                0,
            )
        };

        let eff_len = (nucleus_end - eff_nucleus_start).min(3);
        let eff_slice = &eff_nuc_slice[..eff_len];

        if let Some(target_in_nucleus) = nucleus_tone_target(eff_slice) {
            // Traditional orthography: place the tone on the FIRST vowel of
            // `oa`, `oe`, `uy` (e.g. `hóa` instead of `hoá`, `hòe` instead
            // of `hoè`, `tùy` instead of `tuỳ`). Modern orthography (the
            // table default) places it on the second vowel. The `qu-` and
            // `gi-` onsets already strip their glide vowel above, so `uy`
            // here is the standalone case (e.g. `tùy`), not `quy`.
            if !self.enable_modern_orthography && target_in_nucleus == 1 {
                let is_traditional_diphthong =
                    matches!(eff_slice, ['o', 'a'] | ['o', 'e'] | ['u', 'y']);
                if is_traditional_diphthong {
                    return Some(eff_nucleus_start + tone_offset);
                }
            }
            return Some(eff_nucleus_start + target_in_nucleus + tone_offset);
        }

        if eff_nucleus_start < nucleus_end {
            Some(nucleus_end - 1)
        } else if nucleus_start < nucleus_end {
            Some(nucleus_end - 1)
        } else {
            None
        }
    }

    #[inline]
    fn reapply_tone_after_nucleus_change(&mut self) {
        let (_, nucleus_start, nucleus_end, _) = self.partition_syllable();
        let mut tone_val: Option<u8> = None;
        let mut old_carrier: Option<usize> = None;
        for i in nucleus_start..nucleus_end {
            let s = self.buf.get(i);
            if s.flags & F_TONE_SET != 0 {
                tone_val = Some(s.tone);
                old_carrier = Some(i);
                break;
            }
        }

        let Some(tv) = tone_val else {
            return;
        };
        let new_carrier = self.tone_carrier_idx();

        if new_carrier == old_carrier {
            return;
        }

        if let Some(oc) = old_carrier {
            let s = self.buf.get_mut(oc);
            s.flags &= !F_TONE_SET;
            s.tone = 0;
            s.recompute_out();
        }

        if let Some(new_carrier) = self.tone_carrier_idx() {
            let s = self.buf.get_mut(new_carrier);
            s.tone = tv;
            s.flags |= F_TONE_SET;
            s.recompute_out();
        }
    }

    /// In traditional orthography, the tone for `oa`/`oe`/`uy` diphthongs is
    /// placed on the first vowel (e.g. "hoá" not "hóa"). This first-vowel
    /// placement is only correct for **open syllables** (no coda). When the
    /// syllable has any coda, the tone must go on the second vowel in both
    /// orthography modes — "đoán" not "đóan", "hoàn" not "hóan", "hoạt" not
    /// "họat".
    ///
    /// Called from `render_out_buf` after the syllable structure is known,
    /// because the tone is placed when the tone key is pressed (before the
    /// coda is typed).
    #[inline]
    fn apply_coda_tone_rule(&mut self) {
        // /uə/ rhyme resolution: plain [u,o] + consonant coda + tone → the
        // second vowel takes the circumflex and carries the tone ("thuocs" →
        // "thuốc", "muons" → "muốn", "nguonf" → "nguồn"). Without this, the
        // tone lands on the u and produces fake Vietnamese ("thúoc", "múon").
        // Open /uə/ is written "uơ" (horn) and is handled by the w-modifier.
        self.resolve_uo_rhyme_tone();
        // -ech rhyme resolution: plain [e] + coda "ch" + tone → the e takes
        // the circumflex ("lechj" → "lệch", "kechs" → "ếch" — the -ech rhyme
        // is always written "êch" when toned).
        self.resolve_ech_rhyme_tone();
        // Centering-diphthong resolution: plain-vowel nuclei whose written
        // form needs a circumflex on the centering vowel — [i,e]/[y,e]→iê/yê
        // ("tiens" → "tiến", "quyens" → "quyến"), [u,e]→uê ("hues" → "huế",
        // "khuechs" → "khuếch"), [u,a]→uâ ("tuans" → "tuấn" — closed only,
        // open "ua" stays), [e,u]→êu ("deux" → "đều"), and the plain
        // triphthongs ("kieux" → "kiểu", "quyets" → "quyết",
        // "cuois" → "cuối", "khuays" → "khuấy").
        self.resolve_centering_rhyme_tone();

        if self.enable_modern_orthography {
            return; // Modern mode already places tone on second vowel.
        }

        let n = self.buf.len();
        if n < 3 {
            return; // Need at least 2 vowels + coda (onset is optional).
        }

        let (_, nucleus_start, nucleus_end, coda_start) = self.partition_syllable();

        // Must have a coda (any consonant after the nucleus).
        if coda_start >= n {
            return;
        }

        // Nucleus must be a 2-vowel diphthong: oa, oe, uy.
        let nucleus_len = nucleus_end.saturating_sub(nucleus_start);
        if nucleus_len != 2 {
            return;
        }
        let nuc0 = self.buf.get(nucleus_start).base_no_tone();
        let nuc1 = self.buf.get(nucleus_start + 1).base_no_tone();
        let is_traditional_diphthong = matches!(
            (nuc0, nuc1),
            ('o', 'a') | ('o', 'e') | ('u', 'y') | ('o', 'o')
        );
        if !is_traditional_diphthong {
            return;
        }

        // Check if tone is on the first vowel (traditional placement).
        let first = self.buf.get(nucleus_start);
        if first.flags & F_TONE_SET == 0 {
            return; // No tone on first vowel — nothing to fix.
        }

        // Move tone from first vowel to second vowel (modern placement).
        let tone = first.tone;
        {
            let s = self.buf.get_mut(nucleus_start);
            s.flags &= !F_TONE_SET;
            s.tone = 0;
            s.recompute_out();
        }
        {
            let s = self.buf.get_mut(nucleus_start + 1);
            s.tone = tone;
            s.flags |= F_TONE_SET;
            s.recompute_out();
        }
    }

    /// /uə/ rhyme resolution: plain `[u, o]` + consonant coda + tone → the
    /// second vowel takes the circumflex and carries the tone.
    ///
    /// The /uə/ rhyme is written "uô" when closed (`thuốc`, `muốn`, `nguồn`,
    /// `thuộc` — always with the circumflex on the second vowel) and "uơ"
    /// (horn) when open (`thuở`). The plain-o form only exists transiently
    /// while typing, so once a tone is applied to a closed [u,o] syllable the
    /// o must resolve to ô. Runs at render time so both keystroke orders work
    /// ("thuocs" and "thuocos").
    #[inline]
    fn resolve_uo_rhyme_tone(&mut self) {
        let n = self.buf.len();
        if n < 3 {
            return; // Need at least u + o + coda (onset optional).
        }

        let (_, nucleus_start, nucleus_end, coda_start) = self.partition_syllable();

        // Must have a consonant coda.
        if coda_start >= n {
            return;
        }

        // Nucleus must be exactly the plain pair [u, o] (no horn/circumflex).
        if nucleus_end - nucleus_start != 2 {
            return;
        }
        let u_syl = self.buf.get(nucleus_start);
        let o_syl = self.buf.get(nucleus_start + 1);
        if u_syl.base != b'u' || o_syl.base != b'o' {
            return;
        }
        if u_syl.flags & (F_HORN | F_CIRCUMFLEX) != 0 || o_syl.flags & (F_HORN | F_CIRCUMFLEX) != 0
        {
            return;
        }

        // A tone must already be set on one of the two vowels.
        let (tone, from_idx) = if u_syl.flags & F_TONE_SET != 0 {
            (u_syl.tone, nucleus_start)
        } else if o_syl.flags & F_TONE_SET != 0 {
            (o_syl.tone, nucleus_start + 1)
        } else {
            return;
        };

        // Move the tone to the o and give it the circumflex
        // ("uố", "uồ", "uộ" — thuốc, muốn, nguồn, thuộc, buồng).
        if from_idx != nucleus_start + 1 {
            let s = self.buf.get_mut(from_idx);
            s.flags &= !F_TONE_SET;
            s.tone = 0;
            s.recompute_out();
        }
        let s = self.buf.get_mut(nucleus_start + 1);
        s.flags |= F_CIRCUMFLEX | F_TONE_SET;
        s.tone = tone;
        s.recompute_out();
    }

    /// -ech rhyme resolution: plain [e] + coda "ch" + tone → the e takes the
    /// circumflex and keeps the tone. The -ech rhyme is always written "êch"
    /// when toned ("lechj" → "lệch", "kechs" → ếch-style — sắc/nặng only,
    /// 20+ words in the 22k list); toneless "ech" is a transient, and
    /// huyền/hỏi/ngã with -ch do not occur (the tone-coda rule rejects them).
    #[inline]
    fn resolve_ech_rhyme_tone(&mut self) {
        let n = self.buf.len();
        if n < 3 {
            return; // Need at least e + ch (onset optional).
        }

        let (_, nucleus_start, nucleus_end, coda_start) = self.partition_syllable();

        if coda_start >= n || nucleus_end - nucleus_start != 1 || nucleus_start + 1 != coda_start {
            return;
        }
        let e_syl = self.buf.get(nucleus_start);
        if e_syl.base != b'e' || e_syl.flags & (F_HORN | F_CIRCUMFLEX) != 0 {
            return;
        }
        // Coda must be exactly "ch".
        if n - coda_start != 2
            || self.buf.get(coda_start).base != b'c'
            || self.buf.get(coda_start + 1).base != b'h'
        {
            return;
        }
        // Only resolve when a tone is present (sắc/nặng pass the tone-coda
        // rule; other tones fall through to passthrough untouched).
        if e_syl.flags & F_TONE_SET == 0 {
            return;
        }
        let s = self.buf.get_mut(nucleus_start);
        s.flags |= F_CIRCUMFLEX;
        s.recompute_out();
    }

    /// Centering-diphthong resolution: a plain-vowel nucleus carrying a tone
    /// resolves its centering vowel to the circumflexed written form, exactly
    /// as if the user had typed the doubled vowel ("tien" + 's' ≡ "tieen" +
    /// 's'). Without this, the tone key falls back to a literal consonant for
    /// the most common Vietnamese rhymes typed the UniKey way: iên, iêng,
    /// iêt, yên, uân, uê, êu, iêu, yêu, uyê, uôi, uây, uêu.
    ///
    /// The resolving vowel and whether a consonant coda is required:
    ///   - `[i,e]` / `[y,e]` → ê on the second vowel — open or closed
    ///     ("tiến", "quyến"; an open "viế" is not a written word but stays
    ///     consistent with the resolved interpretation).
    ///   - `[u,e]` → ê — open or closed ("huế", "khuếch").
    ///   - `[u,a]` → â — closed only; open "ua" is a legitimate plain
    ///     diphthong ("múa" must not become "muâ").
    ///   - `[e,u]` → ê on the first vowel — êu is always open ("đều").
    ///   - Triphthongs: `[i,e,u]`/`[y,e,u]` → ê on the middle vowel
    ///     ("kiểu", "yếu"), `[u,y,e]` → ê on the last ("quyết"),
    ///     `[u,o,i]` → ô on the middle ("cuối"), `[u,a,y]`/`[u,e,u]` → â/ê
    ///     on the middle ("khuấy", "khểu") — all open.
    ///
    /// Runs at render time so both keystroke orders resolve ("tiens" and
    /// "tiesn" alike), and never fires without a tone in the nucleus — an
    /// untoned "tien" keeps rendering literally.
    #[inline]
    fn resolve_centering_rhyme_tone(&mut self) {
        let n = self.buf.len();
        if n < 2 {
            return;
        }
        let (_, nucleus_start, nucleus_end, coda_start) = self.partition_syllable();
        let nuc_len = nucleus_end - nucleus_start;
        if nuc_len < 2 || nuc_len > 3 {
            return;
        }
        let closed = coda_start < n;
        let v0 = self.buf.get(nucleus_start).base;
        let v1 = self.buf.get(nucleus_start + 1).base;
        let v2 = if nuc_len == 3 {
            self.buf.get(nucleus_start + 2).base
        } else {
            0
        };
        // `target` = the nucleus index that takes the circumflex + tone.
        let target: usize = match (v0, v1, v2) {
            (b'i' | b'y', b'e', 0) => 1,
            (b'u', b'e', 0) => 1,
            (b'u', b'a', 0) if closed => 1,
            (b'e', b'u', 0) => 0,
            (b'i' | b'y', b'e', b'u') => 1,
            (b'u', b'y', b'e') => 2,
            (b'u', b'o', b'i') | (b'u', b'a', b'y') | (b'u', b'e', b'u') => 1,
            _ => return,
        };
        // All nucleus vowels must still be plain — a horn or circumflex
        // already present means the nucleus resolved earlier or is a
        // different rhyme entirely.
        for i in nucleus_start..nucleus_end {
            if self.buf.get(i).flags & (F_HORN | F_CIRCUMFLEX) != 0 {
                return;
            }
        }
        // A tone must be set on one of the nucleus vowels.
        let mut tone = None;
        for i in nucleus_start..nucleus_end {
            let s = self.buf.get(i);
            if s.flags & F_TONE_SET != 0 {
                tone = Some((s.tone, i));
                break;
            }
        }
        let Some((tone_val, from_idx)) = tone else {
            return;
        };
        let target_idx = nucleus_start + target;
        if from_idx != target_idx {
            let s = self.buf.get_mut(from_idx);
            s.flags &= !F_TONE_SET;
            s.tone = 0;
            s.recompute_out();
        }
        let s = self.buf.get_mut(target_idx);
        s.flags |= F_CIRCUMFLEX | F_TONE_SET;
        s.tone = tone_val;
        s.recompute_out();
    }
}
