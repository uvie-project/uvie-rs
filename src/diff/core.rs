//! Core feed pipeline and backspace rebuild helpers for diff mode.
//!
//! `feed_diff_core` is the forward pipeline shared by the public `feed_diff`
//! (which appends to `key_log` first) and by `rebuild_composing` (which does
//! not). Keeping the append out of the core lets backspace replay keystrokes
//! without double-appending to the log.

use crate::buffers::{CharVec, new_out_buffer};
use crate::composing::Composable;
use crate::diff::Diffable;
use crate::engine::UltraFastViEngine;

impl UltraFastViEngine {
    pub(crate) fn feed_diff_core(&mut self, ch: char) -> (usize, &str) {
        // Word boundary: commit composing word, clear state, return char directly.
        if Self::is_word_boundary(ch) {
            // English dictionary override: if the full word matches a known
            // English word, replace the Vietnamese transform with the raw
            // English word + the boundary char.
            if self.enable_english_override
                && !self.diff.word_raw.is_empty()
                && crate::tables::is_english_override(&self.diff.word_raw)
            {
                // Build full on-screen text (Vietnamese) in a stack buffer.
                let mut full_screen = crate::buffers::new_out_buffer();
                let _ = full_screen.push_str(&self.diff.diff_committed);
                let _ = full_screen.push_str(&self.diff.prev_rendered);

                // Build target: raw English word (preserve case) + boundary char.
                let mut target = crate::buffers::new_out_buffer();
                for &c in self.diff.word_raw.iter() {
                    let _ = target.push(c);
                }
                let _ = target.push(ch);

                // Record the committed word for post-commit editing: on
                // screen it is the raw English word.
                let entry_raw = self.diff.word_raw.clone();
                let entry_rendered = target.clone();
                self.diff.push_committed(entry_raw, entry_rendered);

                // Clear all state.
                self.buf.clear();
                self.raw_len = 0;
                self.out_buf.clear();
                self.diff.clear();

                // Diff from full_screen → target.
                let (bs, _) = Self::diff_into(&full_screen, &target, &mut self.diff.diff_suffix);
                return (bs, &self.diff.diff_suffix);
            }

            // Record the committed word for post-commit editing: raw
            // keystrokes + the exact text on screen.
            let entry_raw = self.diff.word_raw.clone();
            let mut entry_rendered = crate::buffers::new_out_buffer();
            let _ = entry_rendered.push_str(&self.diff.diff_committed);
            let _ = entry_rendered.push_str(&self.diff.prev_rendered);
            self.diff.push_committed(entry_raw, entry_rendered);

            self.buf.clear();
            self.raw_len = 0;
            self.out_buf.clear();
            self.diff.clear();
            let _ = self.diff.diff_suffix.push(ch);
            return (0, &self.diff.diff_suffix);
        }

        // Safety valve: buffer full - commit, start fresh.
        if self.diff.raw_chars.is_full() {
            self.render_out_buf();
            let _ = self.diff.diff_committed.push_str(&self.out_buf);
            // The committed word stays on screen — do NOT backspace it.
            // The new character is appended after it. Returning bs=0 keeps
            // the on-screen text consistent with diff_committed.
            self.buf.clear();
            self.raw_len = 0;
            self.out_buf.clear();
            self.diff.raw_chars.clear();
            self.diff.key_log.clear();
            self.diff.word_raw.clear();
            // The word outgrew the 24-keystroke raw buffer: its prefix is now
            // only on screen (committed above), so the raw keystrokes no
            // longer describe the whole word. Hosts must not offer a literal
            // restore for it (see `word_raw_string`).
            self.diff.word_raw_overflowed = true;
            self.diff.english_fallback = None;
            self.diff.prev_inner_render.clear();
            self.diff.last_valid_raw_len = 0;
            self.diff.last_valid_coda_start = 0;
            self.diff.last_valid_out.clear();
            let _ = self.diff.raw_chars.try_push(ch);
            let _ = self.diff.key_log.try_push(ch);
            let _ = self.diff.word_raw.try_push(ch);
            self.feed(ch);
            // Swap out_buf into prev_rendered (zero-alloc).
            let new_composed = core::mem::take(&mut self.out_buf);
            self.diff.diff_suffix.clear();
            let _ = self.diff.diff_suffix.push_str(&new_composed);
            self.diff.prev_rendered.clear();
            let _ = self.diff.prev_rendered.push_str(&new_composed);
            self.diff.prev_inner_render.clear();
            let _ = self.diff.prev_inner_render.push_str(&new_composed);
            // Clear snapshots — the previous word's snapshots are stale.
            for s in &mut self.diff.snapshots {
                *s = None;
            }
            self.diff.snapshot_count = 0;
            return (0, &self.diff.diff_suffix);
        }

        let raw_len_before = self.raw_len;
        let _ = self.diff.raw_chars.try_push(ch);
        self.feed(ch);
        let raw_len_after = self.raw_len;

        // Double-tone-cancel detection.
        // NOTE: `key_log` is intentionally NOT truncated here. It keeps the full
        // keystroke sequence (including the cancelling key) so that a later
        // backspace can replay it and re-trigger the cancel faithfully. Only
        // `raw_chars` (the live, lossy buffer) is swap+truncate'd.
        let double_cancel_fired =
            raw_len_after == raw_len_before && !self.diff.raw_chars.is_empty();
        if double_cancel_fired {
            let last_idx = self.diff.raw_chars.len() - 1;
            if last_idx >= 1 {
                self.diff.raw_chars.swap(last_idx - 1, last_idx);
                self.diff.raw_chars.truncate(last_idx);
                // Update engine's raw_len to match the modified diff.raw_chars
                self.raw_len = self.diff.raw_chars.len();
            }
            self.diff.last_valid_raw_len = 0;
            self.diff.last_valid_coda_start = 0;
            self.diff.last_valid_out.clear();
        }

        // Swap out_buf into new_composed (zero-alloc, avoids String::clone).
        let mut new_composed = new_out_buffer();
        core::mem::swap(&mut new_composed, &mut self.out_buf);
        let is_now_raw = Self::is_raw_passthrough_slice(&self.diff.raw_chars, &new_composed);

        if !is_now_raw {
            self.diff.last_valid_raw_len = self.diff.raw_chars.len();
            self.diff.last_valid_coda_start = Self::raw_coda_start(&self.diff.raw_chars);
            self.diff.last_valid_out.clear();
            let _ = self.diff.last_valid_out.push_str(&new_composed);
        }

        // Optimistic display: show coda consonant appended to valid Vietnamese.
        // Only use it when the valid Vietnamese syllable had no coda yet;
        // otherwise the screen and the engine's true state diverge, causing ghost characters.
        //
        // Branch prediction: is_optimistic is RARE (only when a single
        // consonant is appended after a valid Vietnamese syllable with no
        // coda). We check the cheap conditions first and short-circuit
        // before touching scratch buffers.
        let ch_is_tone = Self::is_tone_key_in_mode(ch, self.mode);

        // Fast check: if not raw or last_valid_out empty, skip optimistic entirely.
        let is_optimistic = is_now_raw
            && !ch_is_tone
            && !self.diff.last_valid_out.is_empty()
            && Self::is_single_consonant_appended_slice(
                &self.diff.raw_chars,
                self.diff.last_valid_raw_len,
            )
            && self.diff.last_valid_coda_start == self.diff.last_valid_raw_len;

        // Build display_composed: on the common path (not optimistic), we
        // diff directly from new_composed — no scratch buffer needed.
        // Only build scratch_display when optimistic (rare).
        let display_composed: &str = if is_optimistic {
            self.diff.scratch_display.clear();
            let _ = self
                .diff
                .scratch_display
                .push_str(&self.diff.last_valid_out);
            let _ = self.diff.scratch_display.push(ch);
            &self.diff.scratch_display
        } else {
            &new_composed
        };

        // Diff baseline: prev_rendered is used as-is (no clone needed — we
        // diff from it, then overwrite it below).
        self.diff.prev_inner_render.clear();
        let _ = self.diff.prev_inner_render.push_str(&new_composed);

        // V-C-V boundary detection.
        // RARE: only fires when a vowel starts a new syllable after a consonant.
        // Check cheap conditions first for branch prediction.
        let ch_is_vowel = Self::is_ascii_vowel(ch as u8);
        if is_now_raw
            && ch_is_vowel
            && !self.diff.last_valid_out.is_empty()
            && self.diff.last_valid_raw_len < self.diff.raw_chars.len()
        {
            let split = Self::find_split_point(&self.diff.raw_chars);
            // Vietnamese orthography rule: a syllable boundary inside a written
            // token must fall on a consonant (the next syllable's onset). A
            // split directly after a nucleus with no intervening consonant
            // (vowel-to-vowel hiatus, e.g. "theeo" → "thê"|"o") never occurs
            // in Vietnamese — any V-V sequence inside a token must be a single
            // nucleus. Hiatus marks a non-Vietnamese token, so keep composing
            // raw instead of committing a syllable pair (fixes "theeo" →
            // "thêo" and "ressearch" → "rếarch").
            if split > 0 && split < self.diff.raw_chars.len() - 1 {
                let committed_raw: CharVec<24> =
                    self.diff.raw_chars[..split].iter().copied().collect();
                let new_syl_raw: CharVec<24> =
                    self.diff.raw_chars[split..].iter().copied().collect();

                let committed_out =
                    Self::rerender_chars(&committed_raw, self.mode, self.is_simple_telex);

                let _ = self.diff.diff_committed.push_str(&committed_out);

                // Restart engine with new syllable.
                self.buf.clear();
                self.raw_len = 0;
                self.out_buf.clear();
                for &c in new_syl_raw.iter() {
                    self.feed(c);
                }
                // Swap out_buf into new_composed2 (zero-alloc).
                let new_composed2 = core::mem::take(&mut self.out_buf);

                // Build full_screen in scratch_optimistic (reused buffer).
                self.diff.scratch_optimistic.clear();
                let _ = self.diff.scratch_optimistic.push_str(&committed_out);
                let _ = self.diff.scratch_optimistic.push_str(&new_composed2);
                let (bs, _) = Self::diff_into(
                    &self.diff.prev_rendered,
                    &self.diff.scratch_optimistic,
                    &mut self.diff.diff_suffix,
                );

                self.diff.raw_chars = new_syl_raw;
                // Mirror the composing portion into `key_log` so backspace can
                // rebuild exactly this syllable. The committed portion's
                // keystrokes are dropped from the log (its rendered form lives
                // on in `diff_committed`).
                self.diff.key_log = self.diff.raw_chars.iter().copied().collect();
                // CRITICAL FIX: Sync raw_len with diff.raw_chars after V-C-V split
                // Without this, backspace() will use wrong indices when replaying keystrokes
                self.raw_len = self.diff.raw_chars.len();
                self.diff.prev_rendered.clear();
                let _ = self.diff.prev_rendered.push_str(&new_composed2);
                self.diff.prev_inner_render.clear();
                let _ = self.diff.prev_inner_render.push_str(&new_composed2);
                let is_new_raw =
                    Self::is_raw_passthrough_slice(&self.diff.raw_chars, &new_composed2);
                if is_new_raw {
                    self.diff.last_valid_raw_len = 0;
                    self.diff.last_valid_coda_start = 0;
                    self.diff.last_valid_out.clear();
                } else {
                    self.diff.last_valid_raw_len = self.diff.raw_chars.len();
                    self.diff.last_valid_coda_start = Self::raw_coda_start(&self.diff.raw_chars);
                    self.diff.last_valid_out.clear();
                    let _ = self.diff.last_valid_out.push_str(&new_composed2);
                }
                return (bs, &self.diff.diff_suffix);
            }
        }

        // Normal path: diff from prev_rendered → display_composed.
        // No clone needed — we diff from prev_rendered then overwrite it.
        let (bs, _) = Self::diff_into(
            &self.diff.prev_rendered,
            display_composed,
            &mut self.diff.diff_suffix,
        );
        self.diff.prev_rendered.clear();
        let _ = self.diff.prev_rendered.push_str(display_composed);
        (bs, &self.diff.diff_suffix)
    }

    /// Abandon a sticky English passthrough and re-render the whole word as
    /// Vietnamese.
    ///
    /// Called when the dictionary override committed a word whose keystrokes
    /// the pipeline had collapsed (a doubled Telex key cancelled a tone —
    /// "mass" is raw "mass" but the pipeline's raw is "mas") and the tail
    /// typed after it can no longer become a dictionary word. The saved
    /// pre-override pipeline state is restored, the tail is replayed through
    /// the live pipeline, and the on-screen raw English text is diffed to
    /// the resulting Vietnamese render — so "masst" → "mast" and "masster"
    /// → "master" instead of the literal keystrokes.
    ///
    /// The replay goes through `feed_diff`, so a word that becomes a
    /// dictionary word again mid-tail (e.g. "massive" + "x") re-enters the
    /// sticky path exactly as if it had been typed that way.
    pub(crate) fn fallback_to_vietnamese(&mut self) -> (usize, &str) {
        let Some(fb) = self.diff.english_fallback.take() else {
            // No saved state (the caller only calls this when one exists):
            // keep the raw tail.
            self.diff.diff_suffix.clear();
            return (0, &self.diff.diff_suffix);
        };

        // Text currently on screen: the raw English word typed so far.
        let mut screen_before = new_out_buffer();
        let _ = screen_before.push_str(&self.diff.diff_committed);
        let _ = screen_before.push_str(&self.diff.prev_rendered);

        // Keystrokes typed after the dictionary word (includes the key that
        // triggered this fallback — `word_raw` already holds it).
        let tail: CharVec<24> = self
            .diff
            .word_raw
            .iter()
            .skip(fb.word_len)
            .copied()
            .collect();

        // Restore the pipeline exactly as the override interrupted it.
        self.buf = fb.snap.buf;
        self.raw_len = fb.snap.raw_len;
        self.diff.raw_chars = fb.snap.raw_chars;
        self.diff.key_log = fb.snap.key_log;
        self.diff.prev_rendered = fb.snap.prev_rendered;
        self.diff.prev_inner_render = fb.snap.prev_inner_render;
        self.diff.last_valid_raw_len = fb.snap.last_valid_raw_len;
        self.diff.last_valid_coda_start = fb.snap.last_valid_coda_start;
        self.diff.last_valid_out = fb.snap.last_valid_out;
        self.diff.diff_committed = fb.diff_committed;
        self.diff.word_raw.truncate(fb.word_len);
        self.diff.english_sticky = false;

        // Replay the tail through the live pipeline (snapshots, V-C-V splits
        // and the dictionary override behave exactly as in a fresh typing
        // session; the per-keystroke diffs are discarded below).
        for &c in tail.iter() {
            let _ = self.feed_diff(c);
        }

        // Diff the raw text that was on screen to the re-rendered word.
        let mut full = new_out_buffer();
        let _ = full.push_str(&self.diff.diff_committed);
        let _ = full.push_str(&self.diff.prev_rendered);
        let (bs, _) = Self::diff_into(&screen_before, &full, &mut self.diff.diff_suffix);
        (bs, &self.diff.diff_suffix)
    }

    /// Rebuild the composing portion of the engine from a lossless keystroke log.
    ///
    /// Resets the core engine and the diff *composing* state, then replays `log`
    /// through `feed_diff_core`. `diff_committed` and `key_log` are intentionally
    /// preserved: the committed V-C-V prefix is still on screen, and `key_log`
    /// was already updated by the caller (`backspace_diff`).
    ///
    /// Because `log` is always a single composing syllable (the V-C-V split point
    /// starts at a consonant), replaying it cannot re-trigger a V-C-V split, so
    /// `key_log` is not truncated here. Cancels/expansions are re-applied exactly
    /// as in the forward path, converging `raw_chars`, `prev_rendered`,
    /// `last_valid_*` and the core `buf`/`raw`/`raw_len`/`out_buf` to a single
    /// consistent state — eliminating the "lossy replay" ghost-character bug.
    pub(crate) fn rebuild_composing(&mut self, log: &[char]) {
        // Reset the core engine to a clean slate (mirrors `clear()` minus diff).
        self.buf.clear();
        self.raw_len = 0;
        self.out_buf.clear();
        self.committed.clear();
        self.syl_structure.clear();

        // Reset diff composing state only.
        self.diff.raw_chars.clear();
        self.diff.prev_rendered.clear();
        self.diff.prev_inner_render.clear();
        self.diff.last_valid_raw_len = 0;
        self.diff.last_valid_coda_start = 0;
        self.diff.last_valid_out.clear();

        // Replay the surviving keystrokes through the core pipeline. This does
        // not touch `key_log` (the wrapper owns appends; V-C-V/full-buffer cannot
        // fire for a single syllable), so the log stays intact for the next
        // backspace.
        for &c in log {
            let _ = self.feed_diff_core(c);
        }
    }
}
