//! Diff-mode state: snapshot stack and tracking buffers.
//!
//! All fields are stack-allocated (no heap) — see `buffers.rs` for the
//! underlying `CharVec` / `OutBuffer` types.

use crate::buffers::{CharVec, OutBuffer};
use crate::syllable::SylBuf;

/// Snapshot of the composing state after a keystroke, used for O(1) backspace.
/// All fields are stack-allocated (no heap).
#[derive(Clone)]
pub struct ComposingSnapshot {
    pub buf: SylBuf,
    pub raw_len: usize,
    pub raw_chars: CharVec<24>,
    pub key_log: CharVec<24>,
    pub prev_rendered: OutBuffer,
    pub prev_inner_render: OutBuffer,
    pub last_valid_raw_len: usize,
    pub last_valid_coda_start: usize,
    pub last_valid_out: OutBuffer,
}

/// A committed word kept for post-commit editing (arrow-back onto the word
/// and typing a modifier/tone key re-renders it in place — LabanKey-style).
/// All fields are stack-allocated (no heap beyond the OutBuffer's inline
/// storage on no_std / String on std).
#[derive(Clone)]
pub struct CommittedWord {
    /// Raw keystrokes of the committed word (lossless, case-preserved).
    pub raw: CharVec<24>,
    /// Rendered text that is on screen for this word.
    pub rendered: OutBuffer,
    /// Separator chars (space / punctuation / Enter) on screen right after
    /// this word — the ones between it and the next newer word, or, for the
    /// newest entry, the trailing ones typed after it. The host reports them
    /// through `note_separator`/`remove_separator`; together with `rendered`
    /// they are the whole caret geometry (`caret_back` from the newest end).
    pub gap: usize,
}

/// Pre-override Vietnamese pipeline state, saved when the English
/// dictionary override commits a word whose keystrokes the pipeline
/// collapsed. A doubled Telex key is the common case: "mass" commits the
/// raw word, but the pipeline's own raw is "mas" — the second `s` cancelled
/// the sắc. Committing pins the rest of the word to raw, so without this
/// state the cancel sequence could never be typed again ("masster" stayed
/// literal instead of rendering "master").
///
/// Restored when a consonant tail can no longer become a dictionary word:
/// the pipeline resumes exactly where the override interrupted it and the
/// whole word is re-rendered as Vietnamese.
#[derive(Clone)]
pub struct EnglishFallback {
    /// Pipeline state right after the keystroke that fired the override.
    pub snap: ComposingSnapshot,
    /// V-C-V committed prefix that was on screen before the override.
    pub diff_committed: OutBuffer,
    /// `word_raw.len()` when the override fired — the tail starts here.
    pub word_len: usize,
}

/// Diff-mode state: tracks what's on screen vs what the engine produced.
pub struct DiffState {
    /// Raw keystroke buffer for feed_diff (chars, not bytes; needed for V-C-V split).
    /// This is the "live" buffer used by the forward path; it is mutated by
    /// double-tone-cancel (swap+truncate) and V-C-V split (replaced by the new
    /// syllable), so it is NOT a faithful record of what the user typed.
    pub raw_chars: CharVec<24>,
    /// Lossless keystroke log for the current *composing* portion only.
    /// Unlike `raw_chars`, this is NEVER truncated/swap'd on double-tone-cancel,
    /// so backspace can faithfully rebuild the composing state by replaying it.
    /// It is reset on word boundary, full-buffer, V-C-V split (set to the new
    /// syllable, mirroring `raw_chars`), commit and reset.
    pub key_log: CharVec<24>,
    /// Full raw keystrokes for the entire current word (never truncated by
    /// V-C-V split or double-tone-cancel). Used for English dictionary
    /// override check at word boundaries. Cleared on word boundary, commit,
    /// reset, and full-buffer overflow.
    pub word_raw: CharVec<24>,
    /// Composing text currently visible on screen (for diffing).
    pub prev_rendered: OutBuffer,
    /// The inner engine's true last render (diff baseline).
    pub prev_inner_render: OutBuffer,
    /// Raw char count at the last valid (non-passthrough) Vietnamese render.
    pub last_valid_raw_len: usize,
    /// Coda start index at the last valid Vietnamese render (used to avoid optimistic display when the syllable already has a coda).
    pub last_valid_coda_start: usize,
    /// Output at the last valid Vietnamese render, used for V-C-V split.
    pub last_valid_out: OutBuffer,
    /// Accumulated auto-committed text from V-C-V splits (diff mode only).
    pub diff_committed: OutBuffer,
    /// Scratch buffer backing the &str returned by feed_diff/backspace_diff.
    pub diff_suffix: OutBuffer,
    /// Scratch buffer for optimistic candidate (avoids heap alloc per keystroke).
    pub scratch_optimistic: OutBuffer,
    /// Scratch buffer for display_composed (avoids heap alloc per keystroke).
    pub scratch_display: OutBuffer,
    /// Snapshot stack: one entry per keystroke, enabling O(1) backspace
    /// instead of O(n) replay. Cleared on word boundary, commit, V-C-V split.
    pub snapshots: [Option<ComposingSnapshot>; 24],
    pub snapshot_count: usize,
    /// Set when the current word outgrew the 24-keystroke `word_raw` buffer
    /// (its prefix was committed mid-word). `word_raw` then describes only
    /// the tail, so hosts must not restore it as the word's literal input.
    pub word_raw_overflowed: bool,
    /// Sticky English passthrough: set when the per-keystroke dictionary
    /// override fires. While set, the rest of the current word passes
    /// through raw — the suffix must not be re-transformed as Vietnamese
    /// (e.g. "perm"+"ission" would render "permision": the double-s
    /// cancel eats a letter and tone keys corrupt the tail). The raw tail
    /// accumulates in `prev_rendered`, keeping the on-screen invariant
    /// (diff_committed + prev_rendered) and the boundary commit path intact.
    /// Cleared on word boundary, commit, reset, and full-word erase.
    pub english_sticky: bool,
    /// Pipeline state saved when the override committed a word whose raw
    /// keystrokes the pipeline collapsed (see `EnglishFallback`). While set,
    /// the sticky tail is checked against the dictionary prefixes and the
    /// word falls back to Vietnamese rendering once it can no longer become
    /// an English word.
    pub english_fallback: Option<EnglishFallback>,
    /// Committed-word ring (oldest → newest), for post-commit editing.
    /// `edit_at_caret_diff` walks all entries to resolve the caret offset and
    /// updates the target IN PLACE (newer entries stay editable). Cleared on
    /// commit-adjacent invalidation paths (reset, full clear).
    pub edit_history: [Option<CommittedWord>; 8],
    /// Index of the oldest entry in the ring.
    pub edit_history_start: usize,
    /// Number of live entries in the ring.
    pub edit_history_len: usize,
    /// Caret offset (`caret_back`, screen chars from the end of the newest
    /// entry) the host must adopt after the last successful edit. Negative
    /// means the caret is right of the new newest word's end (the edited word
    /// was erased, leaving the caret inside the merged separators).
    pub last_edit_caret_back: isize,
}

impl Default for DiffState {
    fn default() -> Self {
        Self::new()
    }
}

impl DiffState {
    pub const fn new() -> Self {
        Self {
            raw_chars: CharVec::new(),
            key_log: CharVec::new(),
            word_raw: CharVec::new(),
            prev_rendered: OutBuffer::new(),
            prev_inner_render: OutBuffer::new(),
            last_valid_raw_len: 0,
            last_valid_coda_start: 0,
            last_valid_out: OutBuffer::new(),
            diff_committed: OutBuffer::new(),
            diff_suffix: OutBuffer::new(),
            scratch_optimistic: OutBuffer::new(),
            scratch_display: OutBuffer::new(),
            snapshots: [const { None }; 24],
            snapshot_count: 0,
            word_raw_overflowed: false,
            english_sticky: false,
            english_fallback: None,
            edit_history: [const { None }; 8],
            edit_history_start: 0,
            edit_history_len: 0,
            last_edit_caret_back: 0,
        }
    }

    pub fn clear(&mut self) {
        self.raw_chars.clear();
        self.key_log.clear();
        self.word_raw.clear();
        self.prev_rendered.clear();
        self.prev_inner_render.clear();
        self.last_valid_raw_len = 0;
        self.last_valid_coda_start = 0;
        self.last_valid_out.clear();
        self.diff_committed.clear();
        self.diff_suffix.clear();
        self.scratch_optimistic.clear();
        self.scratch_display.clear();
        // Clear snapshots
        for s in &mut self.snapshots {
            *s = None;
        }
        self.snapshot_count = 0;
        self.word_raw_overflowed = false;
        self.english_sticky = false;
        self.english_fallback = None;
        // Clear committed-word history
        for w in &mut self.edit_history {
            *w = None;
        }
        self.edit_history_start = 0;
        self.edit_history_len = 0;
        self.last_edit_caret_back = 0;
    }

    /// Push a committed word onto the ring (oldest evicted when full).
    /// The new entry starts with no separators after it; the host reports the
    /// ones typed after the word through `note_separator`.
    pub fn push_committed(&mut self, raw: CharVec<24>, rendered: OutBuffer) {
        if self.edit_history_len == self.edit_history.len() {
            self.edit_history[self.edit_history_start] = None;
            self.edit_history_start = (self.edit_history_start + 1) % self.edit_history.len();
            self.edit_history_len -= 1;
        }
        let idx = (self.edit_history_start + self.edit_history_len) % self.edit_history.len();
        self.edit_history[idx] = Some(CommittedWord {
            raw,
            rendered,
            gap: 0,
        });
        self.edit_history_len += 1;
    }

    /// Ring slot of the entry at `step` (0 = newest, growing older).
    #[inline]
    fn ring_index(&self, step: usize) -> Option<usize> {
        if step >= self.edit_history_len {
            return None;
        }
        Some((self.edit_history_start + self.edit_history_len - 1 - step) % self.edit_history.len())
    }

    /// Borrow the committed word at `step` (0 = newest).
    #[inline]
    pub(crate) fn committed(&self, step: usize) -> Option<&CommittedWord> {
        let idx = self.ring_index(step)?;
        self.edit_history[idx].as_ref()
    }

    /// Mutably borrow the committed word at `step` (0 = newest).
    #[inline]
    pub(crate) fn committed_mut(&mut self, step: usize) -> Option<&mut CommittedWord> {
        let idx = self.ring_index(step)?;
        self.edit_history[idx].as_mut()
    }

    /// Separator chars on screen right after the newest committed word.
    #[inline]
    pub(crate) fn trailing_gap(&self) -> usize {
        self.committed(0).map_or(0, |w| w.gap)
    }

    /// A separator char (space / punctuation / Enter) was typed right after
    /// the newest committed word. No-op while nothing is committed.
    #[inline]
    pub(crate) fn note_separator(&mut self) {
        if let Some(w) = self.committed_mut(0) {
            w.gap += 1;
        }
    }

    /// A separator char right after the newest committed word was deleted.
    /// Returns false when there is no separator left to account for.
    #[inline]
    pub(crate) fn remove_separator(&mut self) -> bool {
        match self.committed_mut(0) {
            Some(w) if w.gap > 0 => {
                w.gap -= 1;
                true
            }
            _ => false,
        }
    }

    /// Remove the committed word at `step` (0 = newest) and shift the ring so
    /// the remaining entries keep their order. The caller merges the removed
    /// entry's gap into the older neighbor (the separators that surrounded it
    /// become adjacent on screen).
    pub(crate) fn remove_committed(&mut self, step: usize) -> Option<CommittedWord> {
        if step >= self.edit_history_len {
            return None;
        }
        let ring_len = self.edit_history.len();
        let mut removed = None;
        let mut kept: [Option<CommittedWord>; 8] = [const { None }; 8];
        let mut n = 0usize;
        // Oldest → newest so `kept` keeps the ring order.
        for s in (0..self.edit_history_len).rev() {
            let idx = (self.edit_history_start + self.edit_history_len - 1 - s) % ring_len;
            let word = self.edit_history[idx].take();
            if s == step {
                removed = word;
            } else if let Some(word) = word {
                kept[n] = Some(word);
                n += 1;
            }
        }
        for (i, word) in kept.into_iter().take(n).enumerate() {
            self.edit_history[(self.edit_history_start + i) % ring_len] = word;
        }
        for i in n..ring_len {
            self.edit_history[(self.edit_history_start + i) % ring_len] = None;
        }
        self.edit_history_len = n;
        removed
    }

    /// Borrow the newest committed word that has raw keystrokes, skipping
    /// empty entries (a commit with nothing composing records nothing, so
    /// these only come from defensive paths).
    ///
    /// Read by the sentence-context English rule at word commit: the
    /// previous word must be on screen as plain English text.
    pub fn newest_committed(&self) -> Option<&CommittedWord> {
        for step in 0..self.edit_history_len {
            let idx = (self.edit_history_start + self.edit_history_len - 1 - step)
                % self.edit_history.len();
            if let Some(w) = self.edit_history[idx].as_ref() {
                if !w.raw.is_empty() {
                    return Some(w);
                }
            }
        }
        None
    }

    /// Push a snapshot of the current composing state.
    /// Called after each successful feed_diff_core.
    #[inline]
    pub fn push_snapshot(&mut self, snap: ComposingSnapshot) {
        if self.snapshot_count < 24 {
            self.snapshots[self.snapshot_count] = Some(snap);
            self.snapshot_count += 1;
        }
    }

    /// Pop and return the last snapshot, or None if empty.
    #[inline]
    pub fn pop_snapshot(&mut self) -> Option<ComposingSnapshot> {
        if self.snapshot_count == 0 {
            return None;
        }
        self.snapshot_count -= 1;
        self.snapshots[self.snapshot_count].take()
    }
}
