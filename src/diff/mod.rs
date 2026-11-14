//! Diff-based input API and V-C-V syllable splitting.
//!
//! The diff engine wraps the core composing engine and computes minimal
//! (backspace_count, suffix_to_type) instructions for each keystroke.

mod core;
mod state;
mod utils;

pub use state::{ComposingSnapshot, DiffState, EnglishFallback};

use crate::buffers::OutBuffer;
use crate::engine::UltraFastViEngine;
use state::CommittedWord;

/// Where a host caret offset (`caret_back`) lands in the committed-word ring.
enum CaretHit {
    /// Exactly at the end of ring entry `step` (0 = newest).
    WordEnd(usize),
    /// Strictly inside ring entry `step`, `chars_before` rendered chars left
    /// of the caret.
    MidWord(usize, usize),
}

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
    /// the ring — 0 = newest word, then `end(k+1) = end(k) + rendered_len(k)
    /// + gap(k+1)` where `gap` is the separators the host reported after each
    /// word — or land strictly inside a word.
    /// A caret strictly inside a word fires a mid-word edit: the keystroke
    /// sequence is split at the position that produced the on-screen prefix
    /// ending at/before the caret, `ch` is inserted there, and the whole
    /// word is re-rendered through a scratch replay of the pipeline.
    /// Returns (backspaces, forward_deletes, suffix): backspaces erase the
    /// word's prefix part left of the caret, forward_deletes erase the old
    /// tail right of the caret, and the suffix inserts the full re-rendered
    /// word — the caret lands at the edited word's end. `forward_deletes`
    /// is 0 for word-end boundary edits (no tail to rewrite).
    /// The target entry is updated IN PLACE and every other entry survives;
    /// the engine stays idle afterwards, so the host's new caret offset is
    /// `last_caret_back_after_edit()`.
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
        // Word boundary: commit the composing word through the same rules as
        // `commit_diff` (dictionary override, restore-raw, sentence context),
        // record the separator for the caret geometry, then pass the char
        // through. The committed-word ring survives — punctuation typed mid
        // sentence must not destroy the post-commit edit history.
        if Self::is_word_boundary(ch) {
            let (bs, _) = self.commit_diff();
            self.note_separator();
            let _ = self.diff.diff_suffix.push(ch);
            return (bs, &self.diff.diff_suffix);
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
            // General re-open: the extended word is no longer dictionary
            // material and renders as ONE complete Vietnamese syllable, so
            // the raw passthrough is not what the user meant - the dictionary
            // word was only a prefix of the Vietnamese keystroke sequence
            // ("soon"+"g" → "sông", "meet"+"j" → "mệt"). A word whose
            // doubled tone key armed a fallback keeps the fallback path's
            // verdict (it returned above when the tail read as Vietnamese).
            if self.diff.english_fallback.is_none() && self.word_reopens_as_vietnamese() {
                return self.reopen_as_vietnamese();
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
            self.pending_tone = None;
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
        // Nothing composing: there is no word to record. An idle commit (a
        // boundary key pressed after punctuation or a second Space) must not
        // push an empty "phantom" entry — the host measures its caret offset
        // from the real word, so a phantom would shift the geometry by one.
        if self.diff.word_raw.is_empty() && !self.is_composing_diff() {
            self.diff.diff_suffix.clear();
            return (0, &self.diff.diff_suffix);
        }

        // English dictionary override: if the full word matches a known
        // English word, replace the Vietnamese transform with the raw English
        // word. The diff engine computes the backspaces needed to transform
        // what's on screen (diff_committed + prev_rendered) into the raw word.
        if self.enable_english_override
            && !self.diff.word_raw.is_empty()
            && crate::tables::is_english_override(&self.diff.word_raw)
        {
            return self.commit_restoring_raw();
        }

        // Restore-raw rule (LabanKey): a finished token that is not a valid
        // Vietnamese word shows the literal keystrokes instead of the marks
        // the pipeline applied to it ("law" → "lă", "after" → "àtẻ"). Same
        // toggle as the dictionary — the host's "restore English" setting.
        if self.enable_english_override
            && !self.diff.word_raw.is_empty()
            && !self.diff.word_raw_overflowed
            && self.should_restore_raw_on_commit()
        {
            return self.commit_restoring_raw();
        }

        // Sentence-context English rule: a common English word that renders
        // as a valid Vietnamese syllable in isolation ("how" → "hơ",
        // "best" → "bét") is restored when the previous committed word is
        // clearly English ("Hello, how are you?"). Vietnamese context never
        // qualifies, so Vietnamese typing is unaffected.
        if self.enable_english_override
            && !self.diff.word_raw.is_empty()
            && !self.diff.word_raw_overflowed
            && self.should_restore_context_english()
        {
            return self.commit_restoring_raw();
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
        self.pending_tone = None;
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
        // The host's caretBack is the ground truth: it must exactly match a
        // word-end boundary or land strictly inside a word (double spaces,
        // pastes and unseen caret jumps resolve to nothing — passthrough).
        match self.resolve_caret(caret_back)? {
            CaretHit::WordEnd(step) => self.edit_word_end(step, caret_back, ch),
            CaretHit::MidWord(step, chars_before) => self.edit_mid_word(step, chars_before, ch),
        }
    }

    /// Mid-word caret edit: the caret sits strictly inside committed word
    /// `step` (0 = newest), `chars_before` rendered chars left of the caret.
    /// The stored raw keystrokes are split at the keystroke that produced
    /// the on-screen prefix ending at/before the caret, `ch` is inserted
    /// there, and the extended sequence is re-rendered through a scratch
    /// replay so tone placement, V-C-V splits, the English override and the
    /// commit-time restore rules all behave exactly as if the word had been
    /// typed that way. Returns
    /// (backspaces = chars_before, forward_deletes = word tail,
    /// suffix = full new render) so the host can rewrite the whole word
    /// and land the caret at its end.
    fn edit_mid_word(
        &mut self,
        step: usize,
        chars_before: usize,
        ch: char,
    ) -> Option<(usize, usize, &str)> {
        let entry_raw = self.diff.committed(step)?.raw.clone();
        let old_rendered = self.diff.committed(step)?.rendered.clone();
        if entry_raw.is_full() {
            return None;
        }

        // Keystroke→screen mapping: replay the raw word on a scratch engine
        // with identical settings and record the on-screen prefix after
        // each key. The split is the last keystroke whose rendered prefix
        // is preserved verbatim in the committed render and ends at or
        // before the caret.
        let mut scratch = self.scratch_engine(self.english_override());
        let mut rendered = crate::buffers::CharVec::<24>::new();
        for c in old_rendered.chars() {
            let _ = rendered.try_push(c);
        }
        let mut screen = crate::buffers::new_out_buffer();
        let mut split = 0usize;
        // Syllable spans: a V-C-V split commits the first syllable — the
        // jump in `diff_committed` marks a rendered boundary, and the
        // post-split `key_log` is exactly the new syllable's raw, so the
        // raw boundary is (keys so far) - key_log.len().
        let mut boundaries = [(0usize, 0usize); 8];
        let mut nb = 0usize;
        let mut last_cl = 0usize;
        for (i, &c) in entry_raw.iter().enumerate() {
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
            let mut at = entry_raw.len();
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

        let mut new_raw = crate::buffers::CharVec::<24>::new();
        for &c in entry_raw.iter().take(insert_at) {
            let _ = new_raw.try_push(c);
        }
        let _ = new_raw.try_push(ch);
        for &c in entry_raw.iter().skip(insert_at) {
            let _ = new_raw.try_push(c);
        }

        let prev = self.diff.committed(step + 1).cloned();
        let new_rendered = self.render_committed_word(&new_raw, prev.as_ref());
        let tail = old_rendered.chars().count() - chars_before;
        self.diff.diff_suffix.clear();
        let _ = self.diff.diff_suffix.push_str(&new_rendered);

        let entry = self.diff.committed_mut(step)?;
        entry.raw = new_raw;
        entry.rendered = new_rendered;
        // The caret lands at the edited word's end.
        let new_caret_back = self.committed_word_end(step)? as isize;
        self.diff.last_edit_caret_back = new_caret_back;
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

impl UltraFastViEngine {
    /// A separator char (space / punctuation / Enter) was typed right after
    /// the newest committed word: grow its on-screen gap. No-op while nothing
    /// is committed.
    pub fn note_separator(&mut self) {
        self.diff.note_separator();
    }

    /// A separator char right after the newest committed word was deleted.
    /// Returns false when there is no separator left to account for.
    pub fn remove_separator(&mut self) -> bool {
        self.diff.remove_separator()
    }

    /// Separator chars currently on screen right after the newest committed
    /// word (the caret offset 0 is just left of them).
    pub fn trailing_gap(&self) -> usize {
        self.diff.trailing_gap()
    }

    /// Caret offset (`caret_back`, screen chars from the end of the newest
    /// committed word) the host must adopt after the last successful edit.
    /// Negative means the caret is right of the new newest word's end.
    pub fn last_caret_back_after_edit(&self) -> isize {
        self.diff.last_edit_caret_back
    }

    /// Undo the last raw keystroke of the committed word ending at the caret
    /// — the same "undo one keystroke" semantics as composing Backspace, one
    /// step later. `caret_back` must match a word-end boundary; mid-word
    /// carets and an empty ring decline.
    ///
    /// Returns `(backspaces, suffix)` to apply AT the caret (which is at the
    /// word's end): erase `backspaces` chars, then type `suffix`. The entry is
    /// updated in place; when its raw becomes empty the entry is removed and
    /// its gap merges into the older neighbor. The engine stays idle, so the
    /// host's new caret offset is `last_caret_back_after_edit()`.
    pub fn backspace_at_caret_diff(&mut self, caret_back: usize) -> Option<(usize, &str)> {
        if self.is_composing_diff() {
            return None;
        }
        let CaretHit::WordEnd(step) = self.resolve_caret(caret_back)? else {
            return None;
        };
        let old_rendered = self.diff.committed(step)?.rendered.clone();
        let mut new_raw = self.diff.committed(step)?.raw.clone();
        new_raw.pop()?;

        if new_raw.is_empty() {
            // The word is gone: drop the entry and let the older neighbor
            // absorb its separators (they become adjacent on screen).
            let older_gap = self.diff.committed(step + 1).map(|w| w.gap);
            let removed = self.diff.remove_committed(step)?;
            if let Some(older) = self.diff.committed_mut(step) {
                older.gap += removed.gap;
            }
            // Erasing the newest word leaves the caret inside the merged
            // separators (right of the new newest word's end); erasing an
            // older one leaves the absolute offset unchanged.
            self.diff.last_edit_caret_back = match (step, older_gap) {
                (0, Some(gap)) => -(gap as isize),
                (0, None) => 0,
                _ => caret_back as isize,
            };
            let (bs, _) = Self::diff_into(&old_rendered, "", &mut self.diff.diff_suffix);
            return Some((bs, &self.diff.diff_suffix));
        }

        let prev = self.diff.committed(step + 1).cloned();
        let new_rendered = self.render_committed_word(&new_raw, prev.as_ref());
        let (bs, _) = Self::diff_into(&old_rendered, &new_rendered, &mut self.diff.diff_suffix);
        let entry = self.diff.committed_mut(step)?;
        entry.raw = new_raw;
        entry.rendered = new_rendered;
        self.diff.last_edit_caret_back = caret_back as isize;
        Some((bs, &self.diff.diff_suffix))
    }

    /// Append `ch` to the raw keystrokes of committed word `step` (0 =
    /// newest) and re-render it in place. Every other ring entry survives —
    /// newer words are still on screen and stay editable — and the engine
    /// stays idle, so the caret ends at the edited word's end.
    fn edit_word_end(
        &mut self,
        step: usize,
        caret_back: usize,
        ch: char,
    ) -> Option<(usize, usize, &str)> {
        let entry = self.diff.committed(step)?;
        // Extended raw must fit the fixed keystroke buffer.
        if entry.raw.is_full() {
            return None;
        }
        let mut new_raw = entry.raw.clone();
        let _ = new_raw.try_push(ch);
        let old_rendered = entry.rendered.clone();

        let prev = self.diff.committed(step + 1).cloned();
        let new_rendered = self.render_committed_word(&new_raw, prev.as_ref());
        let (bs, _) = Self::diff_into(&old_rendered, &new_rendered, &mut self.diff.diff_suffix);

        let entry = self.diff.committed_mut(step)?;
        entry.raw = new_raw;
        entry.rendered = new_rendered;
        // The caret is at this word's end; the newer entries are untouched.
        self.diff.last_edit_caret_back = caret_back as isize;
        Some((bs, 0, &self.diff.diff_suffix))
    }

    /// Resolve a host caret offset (`caret_back`, screen chars from the end of
    /// the newest committed word) against the ring geometry:
    /// `end(0) = 0`, `end(k+1) = end(k) + rendered_len(k) + gap(k+1)`.
    fn resolve_caret(&self, caret_back: usize) -> Option<CaretHit> {
        let mut end = 0usize;
        for step in 0..self.diff.edit_history_len {
            if caret_back == end {
                return Some(CaretHit::WordEnd(step));
            }
            let entry = self.diff.committed(step)?;
            let rlen = entry.rendered.chars().count();
            // Strictly inside this word: (end, end + rlen).
            if caret_back > end && caret_back < end + rlen {
                return Some(CaretHit::MidWord(step, end + rlen - caret_back));
            }
            // gap(k) counts the separators AFTER word k — the ones between it
            // and the newer word k-1.
            let older = self.diff.committed(step + 1)?;
            end += rlen + older.gap;
        }
        None
    }

    /// Word-end offset of ring entry `step` (0 = newest): the caret distance
    /// from the end of the newest entry back to this word's end.
    fn committed_word_end(&self, step: usize) -> Option<usize> {
        let mut end = 0usize;
        for k in 0..step {
            let entry = self.diff.committed(k)?;
            let older = self.diff.committed(k + 1)?;
            end += entry.rendered.chars().count() + older.gap;
        }
        Some(end)
    }

    /// Render `raw` the way a fresh commit would: replay every keystroke on a
    /// scratch engine with the live options, applying each diff to a screen
    /// buffer, then apply the scratch's own `commit_diff` plan so the
    /// dictionary override and the restore-raw rules behave exactly as at a
    /// real word boundary. `prev` is the older committed neighbor, which the
    /// sentence-context English rule reads.
    fn render_committed_word(&self, raw: &[char], prev: Option<&CommittedWord>) -> OutBuffer {
        let mut scratch = self.scratch_engine(self.english_override());
        if let Some(prev) = prev {
            scratch
                .diff
                .push_committed(prev.raw.clone(), prev.rendered.clone());
        }
        let mut screen = crate::buffers::new_out_buffer();
        for &c in raw {
            let (bs, suffix) = scratch.feed_diff(c);
            for _ in 0..bs {
                screen.pop();
            }
            let _ = screen.push_str(suffix);
        }
        // The word is still being edited in place: only the dictionary
        // applies at this point. The restore-raw / context rules judge a
        // FINISHED word and would turn an intermediate state into literal
        // keys ("viêt" ⌫ + 'j' must give "việ", not "vieej").
        if self.english_override() && crate::tables::is_english_override(raw) {
            let (bs, suffix) = scratch.commit_diff();
            for _ in 0..bs {
                screen.pop();
            }
            let _ = screen.push_str(suffix);
        }
        screen
    }

    /// Commit the current word as its literal raw keystrokes: diff the
    /// on-screen text (V-C-V committed prefix + composing render) to
    /// `word_raw`, clear every bit of composing state, and record the word for
    /// post-commit editing with rendered == raw (so an edit re-renders
    /// raw + key from scratch). Shared by the English dictionary override and
    /// the restore-raw rule.
    fn commit_restoring_raw(&mut self) -> (usize, &str) {
        // Build full on-screen text (Vietnamese) in a stack buffer.
        let mut full_screen = crate::buffers::new_out_buffer();
        let _ = full_screen.push_str(&self.diff.diff_committed);
        let _ = full_screen.push_str(&self.diff.prev_rendered);

        // Build target text (raw keystrokes, preserve case) in a stack buffer.
        let mut target = crate::buffers::new_out_buffer();
        for &c in self.diff.word_raw.iter() {
            let _ = target.push(c);
        }

        // Capture the raw keystrokes BEFORE the clears wipe word_raw.
        let entry_raw = self.diff.word_raw.clone();

        // Clear all state.
        self.buf.clear();
        self.raw_len = 0;
        self.pending_tone = None;
        self.out_buf.clear();
        self.committed.clear();
        self.syl_structure.clear();
        // DiffState::clear() also wipes the committed-word ring; keep the
        // older words editable (a restore is an ordinary word commit).
        let ring = ::core::mem::take(&mut self.diff.edit_history);
        let ring_start = self.diff.edit_history_start;
        let ring_len = self.diff.edit_history_len;
        self.diff.clear();
        self.diff.edit_history = ring;
        self.diff.edit_history_start = ring_start;
        self.diff.edit_history_len = ring_len;

        // Record the word for post-commit editing: on screen it is the raw
        // keystrokes (the transform was replaced).
        self.diff.push_committed(entry_raw, target.clone());

        // Diff from full_screen → target, writing suffix into diff_suffix.
        let (bs, _) = Self::diff_into(&full_screen, &target, &mut self.diff.diff_suffix);
        (bs, &self.diff.diff_suffix)
    }

    /// Word-boundary restore-raw rule: `true` when the finished token is not a
    /// valid Vietnamese word and must show the literal keystrokes. A Vietnamese
    /// word token is one syllable, so a token that needed a V-C-V split inside
    /// it, or whose rendered syllable is impossible/incomplete, is
    /// non-Vietnamese input.
    fn should_restore_raw_on_commit(&self) -> bool {
        // What is on screen for this word: V-C-V committed prefix + render.
        let mut screen = crate::buffers::new_out_buffer();
        let _ = screen.push_str(&self.diff.diff_committed);
        let _ = screen.push_str(&self.diff.prev_rendered);
        let mut raw = crate::buffers::new_out_buffer();
        for &c in self.diff.word_raw.iter() {
            let _ = raw.push(c);
        }
        // Already literal — nothing to restore.
        if screen == raw {
            return false;
        }
        // Sticky English passthrough owns the rest of this word.
        if self.diff.english_sticky {
            return false;
        }
        // Plain-letter differences come from intentional double-key cancels
        // ("masster" → "master", "off" → "of") — those must stay.
        if screen.is_ascii() {
            return false;
        }
        // A V-C-V split inside this token glued several syllables together,
        // which a single Vietnamese word never is.
        if !self.diff.diff_committed.is_empty() {
            return true;
        }
        crate::restore::rendered_syllable_needs_restore(&self.diff.prev_rendered)
    }

    /// Sentence-context English rule: `true` when the finished token is a
    /// common English word that the pipeline marked as Vietnamese AND the
    /// previous committed word is clearly English ("Hello, how are you?" —
    /// "how" → "hơ"). Restored words are recorded with rendered == raw, so
    /// the context chains: the dictionary restores "soon", which lets the
    /// second "as" in "as soon as possible" see an English previous word.
    fn should_restore_context_english(&self) -> bool {
        // Sticky English passthrough already shows the word raw.
        if self.diff.english_sticky {
            return false;
        }
        // Plain-ASCII screens (including intentional double-key cancels)
        // need no restore.
        let mut screen = crate::buffers::new_out_buffer();
        let _ = screen.push_str(&self.diff.diff_committed);
        let _ = screen.push_str(&self.diff.prev_rendered);
        if screen.is_ascii() {
            return false;
        }
        // The raw keystrokes must be a common English word.
        if !crate::tables::is_english_ambiguous_word(&self.diff.word_raw) {
            return false;
        }
        // The previous committed word must be on screen as typed, plain
        // ASCII English: a Vietnamese word ("các", "trời") never qualifies.
        let Some(prev) = self.diff.newest_committed() else {
            return false;
        };
        if !prev
            .rendered
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == '\'')
        {
            return false;
        }
        if prev.raw.len() != prev.rendered.chars().count()
            || !prev
                .raw
                .iter()
                .zip(prev.rendered.chars())
                .all(|(&a, b)| a == b)
        {
            return false;
        }
        if crate::tables::is_english_context_word(&prev.rendered) {
            return true;
        }
        // Longer English words ("soon", "more") come from the dictionary.
        let mut chars = crate::buffers::CharVec::<24>::new();
        for c in prev.rendered.chars() {
            if !chars.try_push(c) {
                return false;
            }
        }
        // A word this rule (or the dictionary) already restored to raw keeps
        // the chain going: "what is best" — "is" was restored, so "best" is too.
        crate::tables::is_english_override(&chars)
            || crate::tables::is_english_ambiguous_word(&chars)
    }
}
