//! Diff-based input API and V-C-V syllable splitting.
//!
//! The diff engine wraps the core composing engine and computes minimal
//! (backspace_count, suffix_to_type) instructions for each keystroke.

mod core;
mod state;
mod utils;

pub use state::{ComposingSnapshot, DiffState, EnglishFallback};

use crate::engine::UltraFastViEngine;
use state::CommittedWord;

/// Diff-mode input API: minimal-edit instructions for each keystroke.
pub trait Diffable {
    fn feed_diff(&mut self, ch: char) -> (usize, &str);
    fn backspace_diff(&mut self) -> (usize, &str);
    fn commit_diff(&mut self) -> (usize, &str);
    fn reset_diff(&mut self);
    fn is_composing_diff(&self) -> bool;
    fn current_composing_diff(&self) -> &str;
    fn committed_text_diff(&self) -> &str;
    fn prev_inner_render_debug(&self) -> &str;
    fn prev_rendered_debug(&self) -> &str;
    /// Re-enter a previously committed word with `ch` appended to its raw
    /// keystrokes, re-render it, and return the minimal edit instructions
    /// (backspaces, suffix) that transform the on-screen word. `caret_back`
    /// is the host's caret distance (screen chars) back to the end of the
    /// newest committed word; it must exactly match a word-end boundary in
    /// the ring (0 = newest word, then + rendered_len + 1 per older word).
    /// A caret strictly inside a word fires a mid-word edit: the keystroke
    /// sequence is split at the position that produced the on-screen prefix
    /// ending at/before the caret, `ch` is inserted there, and the whole
    /// word is re-rendered through the live pipeline.
    /// Returns (backspaces, forward_deletes, suffix): backspaces erase the
    /// word's prefix part left of the caret, forward_deletes erase the old
    /// tail right of the caret, and the suffix inserts the full re-rendered
    /// word — the caret lands at the edited word's end. `forward_deletes`
    /// is 0 for word-end boundary edits (no tail to rewrite).
    /// Returns None when there is no matching boundary or interior, no
    /// committed word, `ch` is a word boundary, or the engine is composing.
    fn edit_at_caret_diff(&mut self, caret_back: usize, ch: char) -> Option<(usize, usize, &str)>;

    /// Mid-word caret edit helper for `edit_at_caret_diff`: `step` is the
    /// committed-ring index (0 = newest), `chars_before` the rendered chars
    /// of that word left of the caret.
    fn edit_mid_word(
        &mut self,
        step: usize,
        chars_before: usize,
        ch: char,
    ) -> Option<(usize, usize, &str)>;
}

impl Diffable for UltraFastViEngine {
    fn feed_diff(&mut self, ch: char) -> (usize, &str) {
        // Word boundary: commit, clear snapshots, return char directly.
        if Self::is_word_boundary(ch) {
            for s in &mut self.diff.snapshots {
                *s = None;
            }
            self.diff.snapshot_count = 0;
            let _ = self.diff.key_log.try_push(ch);
            return self.feed_diff_core(ch);
        }

        // Sticky English passthrough: the dictionary override already fired
        // for a prefix of this word, so the word is English input — the rest
        // of it must pass through raw. Re-transforming the suffix as
        // Vietnamese produced hybrids like "perm"+"ission" → "permision"
        // (double-s cancel eats a letter) and "syst"+"ems" → "systém".
        // The tail accumulates in prev_rendered so the screen invariant
        // (diff_committed + prev_rendered) and the boundary commit path
        // stay intact; key_log stays empty so backspace falls through to
        // the committed-prefix pop path.
        //
        // The tail stays raw only while the word can still become a
        // dictionary word. A saved fallback means the override swallowed a
        // cancel key, so a tail that the engine can read as Vietnamese — a
        // consonant starting a new syllable whose render carries a Vietnamese
        // transform ("mas"+"ter" → "tẻ") — is Vietnamese intent: the
        // pre-override pipeline is restored and the whole word re-rendered
        // ("masster" → "master"). Anything else keeps the raw passthrough:
        // vowel tails merge into the committed syllable and garble the word
        // ("lesser" → "lể"), and English compounds whose tail stays literal
        // ("pass"+"key", "pass"+"code", "loss"+"less") are English intent.
        if self.diff.english_sticky {
            let _ = self.diff.word_raw.try_push(ch);
            if let Some(fb) = self.diff.english_fallback.as_ref() {
                let word_len = fb.word_len;
                // The first key typed after the dictionary word picks the
                // direction: a vowel (or a non-letter) tail is an English
                // continuation and keeps the raw passthrough for good.
                let new_syllable = matches!(
                    self.diff.word_raw.get(word_len),
                    Some(&c)
                        if c.is_ascii_alphabetic()
                            && !Self::is_ascii_vowel(c.to_ascii_lowercase() as u8)
                );
                if !new_syllable {
                    self.diff.english_fallback = None;
                } else if !crate::tables::is_english_prefix(&self.diff.word_raw)
                    && Self::tail_reads_as_vietnamese(
                        &self.diff.word_raw,
                        word_len,
                        self.mode,
                        self.is_simple_telex,
                    )
                    && self.word_renders_plain(&self.diff.word_raw)
                {
                    return self.fallback_to_vietnamese();
                }
            }
            let _ = self.diff.prev_rendered.push(ch);
            self.diff.diff_suffix.clear();
            let _ = self.diff.diff_suffix.push(ch);
            return (0, &self.diff.diff_suffix);
        }

        // Push snapshot of state BEFORE this keystroke (for O(1) backspace).
        // This captures the state that backspace should restore to.
        self.diff.push_snapshot(ComposingSnapshot {
            buf: self.buf.clone(),
            raw_len: self.raw_len,
            raw_chars: self.diff.raw_chars.clone(),
            key_log: self.diff.key_log.clone(),
            prev_rendered: self.diff.prev_rendered.clone(),
            prev_inner_render: self.diff.prev_inner_render.clone(),
            last_valid_raw_len: self.diff.last_valid_raw_len,
            last_valid_coda_start: self.diff.last_valid_coda_start,
            last_valid_out: self.diff.last_valid_out.clone(),
        });

        // Append to key_log and word_raw, then run the core pipeline.
        let _ = self.diff.key_log.try_push(ch);
        let _ = self.diff.word_raw.try_push(ch);
        // Only save pre-keystroke screen state if the dictionary override
        // could possibly fire (word_raw >= 4 chars, the minimum dict word).
        // This avoids 256 bytes of clones on the common path.
        let dict_eligible = self.diff.word_raw.len() >= 4;
        let committed_before = if dict_eligible {
            Some(self.diff.diff_committed.clone())
        } else {
            None
        };
        let prev_before = if dict_eligible {
            Some(self.diff.prev_rendered.clone())
        } else {
            None
        };
        let (bs, _suffix) = self.feed_diff_core(ch);

        // English dictionary override (per-keystroke): if the full word
        // typed so far matches a known English word, show the raw English
        // word instead of the garbled Vietnamese transform. This makes the
        // override visible WHILE typing, not just at the word boundary.
        //
        // The Vietnamese engine state (buf, raw_chars, etc.) is still
        // maintained in parallel — if the user continues typing past the
        // dictionary word (e.g. "characters"), the override stops firing
        // and the Vietnamese transform is shown again.
        if dict_eligible
            && self.enable_english_override
            && crate::tables::is_english_override(&self.diff.word_raw)
        {
            let committed_before = committed_before.unwrap();
            let prev_before = prev_before.unwrap();
            // Build full on-screen text BEFORE this keystroke (the baseline
            // for the diff). Use the pre-feed_diff_core snapshots to avoid
            // double-counting V-C-V committed text.
            let mut full_before = crate::buffers::new_out_buffer();
            let _ = full_before.push_str(&committed_before);
            let _ = full_before.push_str(&prev_before);

            // Build target: full raw English word (preserve original case).
            let mut target = crate::buffers::new_out_buffer();
            for &c in self.diff.word_raw.iter() {
                let _ = target.push(c);
            }

            // Recompute diff from full_before → raw word.
            let (bs2, _) = Self::diff_into(&full_before, &target, &mut self.diff.diff_suffix);

            // Keep the pre-override pipeline state when the commit swallows a
            // cancel key: a dictionary word ending in a doubled tone key
            // ("mass") commits the literal raw, while the pipeline's own raw
            // is the collapsed form ("mas" — the second `s` cancelled the
            // sắc). The commit pins the rest of the word to raw, so without
            // this the cancel sequence could never be typed again ("masster"
            // stayed literal instead of rendering "master"). `key_log` keeps
            // every key of the syllable while `raw_chars` is the live,
            // collapsed buffer, so the shorter buffer proves the key was
            // consumed ("class" keeps both s's literal and is not a cancel).
            if Self::ends_with_doubled_tone_key(&self.diff.word_raw, self.mode)
                && self.diff.raw_chars.len() < self.diff.key_log.len()
            {
                self.diff.english_fallback = Some(EnglishFallback {
                    snap: ComposingSnapshot {
                        buf: self.buf.clone(),
                        raw_len: self.raw_len,
                        raw_chars: self.diff.raw_chars.clone(),
                        key_log: self.diff.key_log.clone(),
                        prev_rendered: self.diff.prev_rendered.clone(),
                        prev_inner_render: self.diff.prev_inner_render.clone(),
                        last_valid_raw_len: self.diff.last_valid_raw_len,
                        last_valid_coda_start: self.diff.last_valid_coda_start,
                        last_valid_out: self.diff.last_valid_out.clone(),
                    },
                    diff_committed: self.diff.diff_committed.clone(),
                    word_len: self.diff.word_raw.len(),
                });
            } else {
                self.diff.english_fallback = None;
            }

            // Commit the English word to diff_committed and clear composing
            // state. This is CRITICAL: if we leave raw_chars/buf intact, the
            // next keystroke's V-C-V split will re-render the committed
            // portion from raw_chars, producing Vietnamese transforms (e.g.
            // "good" → "gô") and causing ghost characters.
            //
            // By committing the word and clearing composing state, the next
            // keystroke starts a fresh syllable. word_raw is preserved for
            // future dict checks (e.g. "goodness" matches later).
            self.diff.diff_committed.clear();
            let _ = self.diff.diff_committed.push_str(&target);
            self.diff.prev_rendered.clear();
            // Clear all composing state so feed_diff_core starts fresh.
            self.diff.raw_chars.clear();
            self.diff.key_log.clear();
            self.diff.prev_inner_render.clear();
            self.diff.last_valid_out.clear();
            self.diff.last_valid_raw_len = 0;
            self.diff.last_valid_coda_start = 0;
            // Clear core engine state (matches reset_diff minus diff.clear()).
            self.buf.clear();
            self.raw_len = 0;
            self.out_buf.clear();
            self.committed.clear();
            self.syl_structure.clear();
            // Clear snapshots — composing state is empty, no replay needed.
            for s in &mut self.diff.snapshots {
                *s = None;
            }
            self.diff.snapshot_count = 0;
            // Further same-word chars pass through raw — the committed
            // prefix signals English intent for the rest of the word.
            self.diff.english_sticky = true;
            return (bs2, &self.diff.diff_suffix);
        }

        // Check if V-C-V split or full-buffer happened: key_log would be
        // shorter than snapshot_count (the pre-keystroke snapshots were
        // for the old, longer composing word). Reset snapshots — the first
        // backspace will use the O(n) replay fallback, which is correct
        // because the "before" state for the split vowel never existed
        // in the forward path.
        if self.diff.key_log.len() < self.diff.snapshot_count {
            for s in &mut self.diff.snapshots {
                *s = None;
            }
            self.diff.snapshot_count = 0;
        }

        (bs, &self.diff.diff_suffix)
    }

    fn backspace_diff(&mut self) -> (usize, &str) {
        // Override state: prev_rendered is the raw English word (not the
        // Vietnamese transform). The engine's internal state (key_log,
        // raw_chars) is inconsistent with prev_rendered due to V-C-V split,
        // so replay would produce garbage. Instead, just pop one char from
        // prev_rendered and word_raw.
        //
        // Fast check: if prev_rendered contains any non-ASCII chars, it's
        // Vietnamese (not override state). This avoids the dict lookup and
        // String allocation on the common backspace path.
        // Sticky passthrough: prev_rendered holds the raw tail typed after
        // the fired dictionary prefix — pop it char by char. Once empty,
        // the empty-key_log path below pops the committed prefix itself.
        if self.diff.english_sticky && !self.diff.prev_rendered.is_empty() {
            self.diff.prev_rendered.pop();
            self.pop_word_raw();
            self.diff.diff_suffix.clear();
            return (1, &self.diff.diff_suffix);
        }

        if self.enable_english_override
            && self.diff.word_raw.len() >= 4
            && self.diff.prev_rendered.is_ascii()
            && crate::tables::is_english_override(&self.diff.word_raw)
        {
            // Compare prev_rendered to word_raw (case-sensitive, since both
            // now preserve original case). Both are ASCII (is_ascii check).
            let prev_bytes = self.diff.prev_rendered.as_bytes();
            let matches = prev_bytes.len() == self.diff.word_raw.len()
                && prev_bytes
                    .iter()
                    .zip(self.diff.word_raw.iter())
                    .all(|(b, c)| *b == *c as u8);
            if matches {
                self.diff.prev_rendered.pop();
                self.pop_word_raw();
                if !self.diff.key_log.is_empty() {
                    self.diff.key_log.pop();
                }
                self.diff.diff_suffix.clear();
                return (1, &self.diff.diff_suffix);
            }
        }

        // No composing keystrokes left → fall back to popping auto-committed text
        // (V-C-V split output or English dict override) one rendered char at a time.
        if self.diff.key_log.is_empty() {
            if !self.diff.diff_committed.is_empty() {
                self.diff.diff_committed.pop();
                // Also pop word_raw to keep dict override tracking in sync.
                // Without this, backspace+retype would leave stale chars in
                // word_raw and the dict check would fail.
                self.pop_word_raw();
                self.diff.diff_suffix.clear();
                // The whole word was erased — leave sticky passthrough so
                // fresh typing is Vietnamese again.
                if self.diff.word_raw.is_empty() {
                    self.diff.english_sticky = false;
                    self.diff.english_fallback = None;
                }
                return (1, &self.diff.diff_suffix);
            }
            self.diff.diff_suffix.clear();
            return (0, &self.diff.diff_suffix);
        }

        // O(1) fast path: pop snapshot and restore state directly.
        // The snapshot was pushed BEFORE the keystroke, so it contains the
        // state that backspace should restore to.
        if let Some(snap) = self.diff.pop_snapshot() {
            // Snapshot the on-screen text before restore.
            let prev = self.diff.prev_rendered.clone();

            // Restore engine state from snapshot.
            self.buf = snap.buf;
            self.raw_len = snap.raw_len;
            self.diff.raw_chars = snap.raw_chars;
            self.diff.key_log = snap.key_log;
            // word_raw is not snapshotted — just pop the last char, matching
            // key_log's pop in the replay path below.
            self.pop_word_raw();
            self.diff.prev_rendered = snap.prev_rendered.clone();
            self.diff.prev_inner_render = snap.prev_inner_render;
            self.diff.last_valid_raw_len = snap.last_valid_raw_len;
            self.diff.last_valid_coda_start = snap.last_valid_coda_start;
            self.diff.last_valid_out = snap.last_valid_out;

            // Diff old vs new composing text.
            let (bs, _) =
                Self::diff_into(&prev, &self.diff.prev_rendered, &mut self.diff.diff_suffix);
            return (bs, &self.diff.diff_suffix);
        }

        // Fallback: O(n) replay path (used when snapshot stack is empty,
        // e.g. after V-C-V split or when snapshots were exhausted).
        let prev = self.diff.prev_rendered.clone();
        self.diff.key_log.pop();
        self.pop_word_raw();
        let log: crate::buffers::CharVec<24> = self.diff.key_log.iter().copied().collect();
        self.rebuild_composing(&log);
        let new = self.diff.prev_rendered.clone();
        let (bs, _) = Self::diff_into(&prev, &new, &mut self.diff.diff_suffix);
        (bs, &self.diff.diff_suffix)
    }

    fn commit_diff(&mut self) -> (usize, &str) {
        // English dictionary override: if the full word matches a known
        // English word, replace the Vietnamese transform with the raw English
        // word. The diff engine computes the backspaces needed to transform
        // what's on screen (diff_committed + prev_rendered) into the raw word.
        if self.enable_english_override
            && !self.diff.word_raw.is_empty()
            && crate::tables::is_english_override(&self.diff.word_raw)
        {
            // Build full on-screen text (Vietnamese) in a stack buffer.
            let mut full_screen = crate::buffers::new_out_buffer();
            let _ = full_screen.push_str(&self.diff.diff_committed);
            let _ = full_screen.push_str(&self.diff.prev_rendered);

            // Build target text (raw English word, preserve case) in a stack buffer.
            let mut target = crate::buffers::new_out_buffer();
            for &c in self.diff.word_raw.iter() {
                let _ = target.push(c);
            }

            // Capture the raw keystrokes BEFORE the clears wipe word_raw.
            let entry_raw = self.diff.word_raw.clone();

            // Clear all state.
            self.buf.clear();
            self.raw_len = 0;
            self.out_buf.clear();
            self.committed.clear();
            self.syl_structure.clear();
            self.diff.clear();
            for s in &mut self.diff.snapshots {
                *s = None;
            }
            self.diff.snapshot_count = 0;

            // Record the word for post-commit editing: on screen it is the
            // raw English word (the override replaced the Vietnamese
            // transform). Pushed AFTER the clears — DiffState::clear()
            // wipes the ring.
            self.diff.push_committed(entry_raw, target.clone());

            // Diff from full_screen → target, writing suffix into diff_suffix.
            let (bs, _) = Self::diff_into(&full_screen, &target, &mut self.diff.diff_suffix);
            return (bs, &self.diff.diff_suffix);
        }

        // Capture the word for post-commit editing before the clears wipe
        // the buffers: raw keystrokes + the exact text on screen (V-C-V
        // committed prefix + composing render).
        let entry_raw = self.diff.word_raw.clone();
        let mut entry_rendered = crate::buffers::new_out_buffer();
        let _ = entry_rendered.push_str(&self.diff.diff_committed);
        let _ = entry_rendered.push_str(&self.diff.prev_rendered);

        self.buf.clear();
        self.raw_len = 0;
        self.out_buf.clear();
        self.committed.clear();
        self.syl_structure.clear();
        self.diff.raw_chars.clear();
        self.diff.key_log.clear();
        self.diff.word_raw.clear();
        self.diff.prev_rendered.clear();
        self.diff.prev_inner_render.clear();
        self.diff.last_valid_raw_len = 0;
        self.diff.last_valid_coda_start = 0;
        self.diff.last_valid_out.clear();
        // A word has just been finalised: the V-C-V auto-committed portion of
        // this word must not survive into the next word, otherwise it leaks onto
        // the following word as ghost characters and corrupts macro matching.
        self.diff.diff_committed.clear();
        self.diff.diff_suffix.clear();
        self.diff.word_raw_overflowed = false;
        self.diff.english_sticky = false;
        self.diff.english_fallback = None;
        // Clear the snapshot stack — stale snapshots from the committed word
        // must not survive, otherwise a backspace after commit would restore
        // state from the previous word, corrupting the engine.
        for s in &mut self.diff.snapshots {
            *s = None;
        }
        self.diff.snapshot_count = 0;
        // Record the word now that the clears are done (push_committed must
        // not be wiped by the diff-state clear above).
        self.diff.push_committed(entry_raw, entry_rendered);
        (0, &self.diff.diff_suffix)
    }

    fn edit_at_caret_diff(&mut self, caret_back: usize, ch: char) -> Option<(usize, usize, &str)> {
        // Word boundaries are commits, not edits — the host routes them to
        // commit_diff. Editing also only applies when idle (nothing composing).
        if Self::is_word_boundary(ch) || self.is_composing_diff() {
            return None;
        }
        let len = self.diff.edit_history_len;
        if len == 0 {
            return None;
        }
        // Walk the ring from the newest word backward, accumulating the
        // on-screen distance to each word-end boundary: boundary 0 = the
        // newest word's end (the anchor), then + rendered_len + 1 boundary
        // char (space/Enter/punct) per older word. The host's caretBack is
        // the ground truth; if it doesn't exactly match a boundary (double
        // spaces, pastes, unseen caret jumps), no edit fires — passthrough.
        let ring_len = self.diff.edit_history.len();
        let mut boundary = 0usize;
        let mut target_step = None;
        let mut midword: Option<(usize, usize)> = None;
        for step in 0..len {
            if caret_back == boundary {
                target_step = Some(step);
                break;
            }
            let idx = (self.diff.edit_history_start + len - 1 - step) % ring_len;
            let entry = self.diff.edit_history[idx].as_ref()?;
            let rlen = entry.rendered.chars().count();
            // Caret strictly inside this word: (boundary, boundary + rlen).
            // `chars_before` = rendered chars of this word left of the caret.
            if caret_back > boundary && caret_back < boundary + rlen {
                midword = Some((step, boundary + rlen - caret_back));
                break;
            }
            boundary += rlen + 1;
        }
        if let Some((step, chars_before)) = midword {
            return self.edit_mid_word(step, chars_before, ch);
        }
        let step = target_step?;
        let target_idx = (self.diff.edit_history_start + len - 1 - step) % ring_len;
        // Extended raw must fit the fixed keystroke buffer.
        if self.diff.edit_history[target_idx].as_ref()?.raw.is_full() {
            return None;
        }
        // Take the target word out of the ring, plus every NEWER entry:
        // they sit after the caret on screen, so their anchor geometry is
        // stale once the target is re-entered as the composing word. The
        // older entries stay (they are still on screen before the caret).
        let entry = self.diff.edit_history[target_idx].take()?;
        let mut older: [Option<CommittedWord>; 8] = [const { None }; 8];
        for i in 0..len - 1 - step {
            let idx = (self.diff.edit_history_start + i) % ring_len;
            older[i] = self.diff.edit_history[idx].take();
        }
        self.diff.edit_history_start = 0;
        self.diff.edit_history_len = 0;

        // Re-render the extended raw word from scratch through the live
        // pipeline (feed_diff), so composing state, V-C-V splits, English
        // override and the snapshot stack all match a fresh typing session.
        let mut new_raw = entry.raw.clone();
        let _ = new_raw.try_push(ch);
        self.reset_diff();
        for &c in new_raw.iter() {
            let _ = self.feed_diff(c);
        }

        // Restore the older committed words (they are still on screen, left
        // of the edited word, and remain editable).
        for w in older.into_iter().flatten() {
            self.diff.push_committed(w.raw, w.rendered);
        }

        // Assemble the new full on-screen word (same math as the commit path).
        let mut full = crate::buffers::new_out_buffer();
        let _ = full.push_str(&self.diff.diff_committed);
        let _ = full.push_str(&self.diff.prev_rendered);

        let (bs, _) = Self::diff_into(&entry.rendered, &full, &mut self.diff.diff_suffix);
        Some((bs, 0, &self.diff.diff_suffix))
    }

    /// Mid-word caret edit: the caret sits strictly inside committed word
    /// `step` (0 = newest), `chars_before` rendered chars left of the caret.
    /// The stored raw keystrokes are split at the keystroke that produced
    /// the on-screen prefix ending at/before the caret, `ch` is inserted
    /// there, and the extended sequence is replayed through feed_diff so
    /// tone placement, V-C-V splits and the English override all behave
    /// exactly as if the word had been typed that way. Returns
    /// (backspaces = chars_before, forward_deletes = word tail,
    /// suffix = full new render) so the host can rewrite the whole word
    /// and land the caret at its end.
    fn edit_mid_word(
        &mut self,
        step: usize,
        chars_before: usize,
        ch: char,
    ) -> Option<(usize, usize, &str)> {
        let len = self.diff.edit_history_len;
        let ring_len = self.diff.edit_history.len();
        let target_idx = (self.diff.edit_history_start + len - 1 - step) % ring_len;
        let entry = self.diff.edit_history[target_idx].as_ref()?;
        if entry.raw.is_full() {
            return None;
        }

        // Keystroke→screen mapping: replay the raw word on a scratch engine
        // with identical settings and record the on-screen prefix after
        // each key. The split is the last keystroke whose rendered prefix
        // is preserved verbatim in the committed render and ends at or
        // before the caret.
        let mut scratch = UltraFastViEngine::new();
        scratch.set_input_method(self.input_method());
        scratch.set_modern_orthography(self.modern_orthography());
        scratch.set_quick_start(self.quick_start());
        scratch.set_quick_telex(self.quick_telex());
        scratch.set_relaxed_coda(self.relaxed_coda());
        scratch.set_english_override(self.english_override());
        let rendered: Vec<char> = entry.rendered.chars().collect();
        let mut screen = String::new();
        let mut split = 0usize;
        // Syllable spans: a V-C-V split commits the first syllable — the
        // jump in `diff_committed` marks a rendered boundary, and the
        // post-split `key_log` is exactly the new syllable's raw, so the
        // raw boundary is (keys so far) - key_log.len().
        let mut boundaries = [(0usize, 0usize); 8];
        let mut nb = 0usize;
        let mut last_cl = 0usize;
        for (i, &c) in entry.raw.iter().enumerate() {
            let (bs, suffix) = scratch.feed_diff(c);
            for _ in 0..bs {
                screen.pop();
            }
            screen.push_str(suffix);
            let cl = scratch.diff.diff_committed.chars().count();
            if cl > last_cl && nb < boundaries.len() {
                boundaries[nb] = ((i + 1).saturating_sub(scratch.diff.key_log.len()), cl);
                nb += 1;
                last_cl = cl;
            }
            let plen = screen.chars().count();
            if plen <= chars_before && screen.chars().zip(rendered.iter()).all(|(a, &b)| a == b) {
                split = i + 1;
            }
        }

        // Tone/modifier keys act on the syllable the caret sits in, not at
        // the keystroke position — inserting 's' mid-coda ("vies|t") can't
        // apply the tone. Snap them to the end of that syllable's raw run
        // so "viet" + caret in "vie" + 's' replays "viets" → "viết".
        // 'd' stays a caret-split literal (it doubles as a normal onset).
        let cl = self.mode.classify[ch as usize];
        let syllable_key = cl & crate::modes::IS_TONE_KEY != 0
            || (cl & crate::modes::IS_MODIFIER != 0 && ch != 'd');
        let insert_at = if syllable_key {
            // First span whose rendered end reaches the caret owns it
            // (caret at a syllable boundary tones the preceding syllable).
            let mut at = entry.raw.len();
            for &(raw_b, rend_b) in boundaries.iter().take(nb) {
                if chars_before <= rend_b {
                    at = raw_b;
                    break;
                }
            }
            at
        } else if split == 0 {
            // No keystroke boundary maps to the caret region (the very
            // first transform rewrote the prefix) — decline rather than
            // insert at a wrong position.
            return None;
        } else {
            split
        };

        // Take the target word plus every NEWER entry out of the ring —
        // same geometry invalidation as the boundary edit.
        let entry = self.diff.edit_history[target_idx].take()?;
        let mut older: [Option<CommittedWord>; 8] = [const { None }; 8];
        for i in 0..len - 1 - step {
            let idx = (self.diff.edit_history_start + i) % ring_len;
            older[i] = self.diff.edit_history[idx].take();
        }
        self.diff.edit_history_start = 0;
        self.diff.edit_history_len = 0;

        let mut new_raw = crate::buffers::CharVec::<24>::new();
        for &c in entry.raw.iter().take(insert_at) {
            let _ = new_raw.try_push(c);
        }
        let _ = new_raw.try_push(ch);
        for &c in entry.raw.iter().skip(insert_at) {
            let _ = new_raw.try_push(c);
        }
        self.reset_diff();
        for &c in new_raw.iter() {
            let _ = self.feed_diff(c);
        }

        for w in older.into_iter().flatten() {
            self.diff.push_committed(w.raw, w.rendered);
        }

        let mut full = crate::buffers::new_out_buffer();
        let _ = full.push_str(&self.diff.diff_committed);
        let _ = full.push_str(&self.diff.prev_rendered);

        let tail = entry.rendered.chars().count() - chars_before;
        self.diff.diff_suffix.clear();
        let _ = self.diff.diff_suffix.push_str(&full);
        Some((chars_before, tail, &self.diff.diff_suffix))
    }

    fn reset_diff(&mut self) {
        // Full reset: clear both the core engine state AND the diff state.
        // Must be consistent with clear() + diff.clear() to avoid stale
        // committed text, syl_structure, or cached partition surviving.
        self.clear();
        self.diff.clear();
    }

    fn is_composing_diff(&self) -> bool {
        // Account for both the live composing keystrokes and any auto-committed
        // V-C-V text that is still on screen; otherwise the host can think the
        // engine is idle while committed text remains, skipping needed resets.
        !self.diff.key_log.is_empty() || !self.diff.diff_committed.is_empty()
    }

    fn current_composing_diff(&self) -> &str {
        &self.diff.prev_rendered
    }

    fn committed_text_diff(&self) -> &str {
        &self.diff.diff_committed
    }

    fn prev_inner_render_debug(&self) -> &str {
        &self.diff.prev_inner_render
    }

    fn prev_rendered_debug(&self) -> &str {
        &self.diff.prev_rendered
    }
}
