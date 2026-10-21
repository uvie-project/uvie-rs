//! Syllable validation and partitioning logic.
//!
//! Validates the current buffer against positive Vietnamese syllable tables
//! and partitions it into onset/nucleus/coda slots.

use crate::engine::UltraFastViEngine;
use crate::modes::IS_VOWEL;
use crate::syllable::{
    F_CAPS, F_CIRCUMFLEX, F_HORN, F_LITERAL, F_TONE_SET, NucleusKind, OnsetKind, Syl, SylStructure,
};
use crate::tables::{
    is_legal_coda, is_legal_nucleus, is_legal_onset, onset_nucleus_compatible,
    rhyme_coda_compatible, tone_allowed_for_coda,
};

/// Syllable validation and structural analysis.
pub(crate) trait SyllableValidator {
    fn is_valid_vietnamese(&self) -> bool;
    fn partition_syllable(&self) -> (usize, usize, usize, usize);
    /// Compute partition from scratch (uncached).
    fn compute_partition(&self) -> (usize, usize, usize, usize);
    fn mark_all_literal(&mut self);
    fn is_vowel_entry(&self, s: &Syl) -> bool;
    fn onset_len(&self) -> usize;
    fn onset_raw_slice(&self) -> &[u8];
    fn update_syl_structure(&mut self);
    fn derive_onset_kind(&self, onset_end: usize) -> OnsetKind;
    fn is_u_glide(&self, idx: usize) -> bool;
}

impl SyllableValidator for UltraFastViEngine {
    #[inline]
    fn is_valid_vietnamese(&self) -> bool {
        let n = self.buf.len();
        if n == 0 {
            return true;
        }

        let (onset_end, nucleus_start, nucleus_end, coda_start) = self.partition_syllable();

        if nucleus_start >= n {
            let onset_raw = &self.raw[..n];
            return is_legal_onset(onset_raw);
        }

        let onset_raw = &self.raw[..onset_end];

        // `partition_syllable` is cached and only guaranteed consistent for
        // the buffer version it was computed for; saturate so a stale or
        // degenerate partition can never underflow (debug) or read a wrapped
        // length (release).
        let nuc_len = nucleus_end.saturating_sub(nucleus_start).min(3);
        let mut nuc = ['\0'; 3];
        for i in 0..nuc_len {
            nuc[i] = self.buf.get(nucleus_start + i).base_no_tone();
        }
        let nuc_slice = &nuc[..nuc_len];

        let coda_len = n.saturating_sub(coda_start);
        let mut coda_raw = [0u8; 4];
        let coda_take = coda_len.min(4);
        for i in 0..coda_take {
            coda_raw[i] = self.buf.get(coda_start + i).base;
        }
        let coda_slice = &coda_raw[..coda_take];

        let mut tone: u8 = 0;
        for i in 0..n {
            let s = self.buf.get(i);
            if s.flags & F_TONE_SET != 0 {
                tone = s.tone;
                break;
            }
        }

        if !is_legal_onset(onset_raw) {
            return false;
        }
        // Onset↔nucleus distribution (c/k/q and g/gh/ng/ngh). Without this,
        // invalid combos like "gh"+"o" pass and a later tone key produces
        // invalid Vietnamese such as "ghost" → "ghót" instead of passthrough.
        if !onset_nucleus_compatible(onset_raw, nuc[0]) {
            return false;
        }
        if !is_legal_nucleus(nuc_slice) {
            return false;
        }
        // Rhyme-level (nucleus + coda) compatibility: closing diphthongs and
        // triphthongs cannot take consonant codas, and monophthongs have
        // per-vowel restrictions (i/y take nh/ch, not ng/c; ơ only m n p t).
        // Without this, "trains" → "tráin" and "bings" → "bíng".
        if !coda_slice.is_empty()
            && !rhyme_coda_compatible(nuc_slice, coda_slice, self.enable_relaxed_coda)
        {
            return false;
        }
        // Transient plain [i,e]/[y,e] nuclei only exist while closed: iê/yê
        // are always-closed written rhymes, so an OPEN "ie"/"ye" can never
        // take a tone key ("iej" stays literal) — "tiens" → "tiến" is the
        // only useful reading.
        if coda_slice.is_empty() && matches!(nuc_slice, ['i', 'e'] | ['y', 'e']) {
            return false;
        }
        if !is_legal_coda(coda_slice, self.enable_relaxed_coda) {
            return false;
        }
        if !tone_allowed_for_coda(coda_slice, tone, self.enable_relaxed_coda) {
            return false;
        }

        true
    }

    #[inline]
    fn partition_syllable(&self) -> (usize, usize, usize, usize) {
        // Fast path: return cached result if buf version matches.
        let version = self.buf.version();
        if self.cached_partition.0 == version {
            return self.cached_partition.1;
        }
        // Cache miss: compute fresh and store.
        let result = self.compute_partition();
        // SAFETY: partition_syllable takes &self but we need to write the
        // cache. We use a raw pointer to bypass the borrow checker. This is
        // sound because:
        // 1. The cache is only read/written here, under &self.
        // 2. The version check ensures we never return stale data.
        // 3. No other &mut self alias exists during this call.
        let this = self as *const Self as *mut Self;
        unsafe {
            (*this).cached_partition = (version, result);
        }
        result
    }

    /// Compute partition from scratch (uncached).
    #[inline]
    fn compute_partition(&self) -> (usize, usize, usize, usize) {
        let n = self.buf.len();

        let mut onset_end = 0;
        while onset_end < n {
            let s = self.buf.get(onset_end);
            if self.is_vowel_entry(s) {
                break;
            }
            onset_end += 1;
        }

        // Special case: `qu` digraph.
        // Check flags excluding F_CAPS and F_LITERAL — uppercase "Qua" or
        // passthrough entries should still be treated as the qu glide.
        if onset_end < n && onset_end > 0 && self.buf.get(onset_end - 1).base == b'q' {
            let next = self.buf.get(onset_end);
            if next.base == b'u' && next.flags & !(F_CAPS | F_LITERAL) == 0 {
                onset_end += 1;
            }
        }

        // Special case: `gi` digraph.
        if onset_end < n && onset_end > 0 && self.buf.get(onset_end - 1).base == b'g' {
            let prev2 = if onset_end >= 2 {
                self.buf.get(onset_end - 2).base
            } else {
                0
            };
            if prev2 != b'n' {
                let next = self.buf.get(onset_end);
                let has_vowel_after_i =
                    onset_end + 1 < n && self.is_vowel_entry(self.buf.get(onset_end + 1));
                if next.base == b'i' && next.flags & !(F_CAPS | F_LITERAL) == 0 && has_vowel_after_i
                {
                    onset_end += 1;
                }
            }
        }

        let nucleus_start = onset_end;
        let mut nucleus_end = nucleus_start;
        while nucleus_end < n {
            let s = self.buf.get(nucleus_end);
            if !self.is_vowel_entry(s) {
                break;
            }
            nucleus_end += 1;
        }

        let coda_start = nucleus_end;
        (onset_end, nucleus_start, nucleus_end, coda_start)
    }

    #[inline]
    fn mark_all_literal(&mut self) {
        for i in 0..self.buf.len() {
            let s = self.buf.get_mut(i);
            s.flags |= F_LITERAL;
            s.flags &= !(F_CIRCUMFLEX | F_HORN | F_TONE_SET);
            s.tone = 0;
            // Preserve uppercase rendering — F_CAPS must still produce an
            // uppercase output char (e.g. "Đ" → "D" after triple-cancel).
            s.out = if s.flags & F_CAPS != 0 {
                (s.base as char).to_ascii_uppercase()
            } else {
                s.base as char
            };
        }
    }

    #[inline]
    fn is_vowel_entry(&self, s: &Syl) -> bool {
        let b = s.base;
        if self.mode.classify[b as usize] & IS_VOWEL != 0 {
            return true;
        }
        if b == b'w' && s.flags & F_HORN != 0 {
            return true;
        }
        false
    }

    #[inline]
    fn onset_len(&self) -> usize {
        let (onset_end, _, _, _) = self.partition_syllable();
        onset_end
    }

    #[inline]
    fn onset_raw_slice(&self) -> &[u8] {
        let onset_len = self.onset_len();
        &self.raw[..onset_len]
    }

    #[inline]
    fn update_syl_structure(&mut self) {
        let (onset_end, _nuc_start, nucleus_end, _coda_start) = self.partition_syllable();
        let onset_kind = self.derive_onset_kind(onset_end);
        let nuc_len = nucleus_end.saturating_sub(onset_end);
        let nucleus_kind = match nuc_len {
            0 => NucleusKind::None,
            1 => NucleusKind::Single,
            2 => NucleusKind::Diphthong,
            _ => NucleusKind::Triphthong,
        };
        self.syl_structure = SylStructure {
            onset_end,
            nucleus_end,
            onset_kind,
            nucleus_kind,
        };
    }

    #[inline]
    fn derive_onset_kind(&self, onset_end: usize) -> OnsetKind {
        match onset_end {
            0 => OnsetKind::None,
            1 => OnsetKind::Single(self.buf.get(0).base),
            2 => OnsetKind::Digraph(self.buf.get(0).base, self.buf.get(1).base),
            3 => OnsetKind::Trigraph,
            _ => OnsetKind::Trigraph,
        }
    }

    #[inline]
    fn is_u_glide(&self, idx: usize) -> bool {
        if idx == 0 {
            return false;
        }
        let prev = self.buf.get(idx - 1);
        prev.base == b'q'
    }
}
