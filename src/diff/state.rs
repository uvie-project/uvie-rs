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
    /// Only the newest entry is used by `edit_newest_diff`; the rest are
    /// kept for future multi-word-back editing. Cleared on commit-adjacent
    /// invalidation paths (reset, full clear) and by `pop_newest_committed`.
    pub edit_history: [Option<CommittedWord>; 8],
    /// Index of the oldest entry in the ring.
    pub edit_history_start: usize,
    /// Number of live entries in the ring.
    pub edit_history_len: usize,
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
    }

    /// Push a committed word onto the ring (oldest evicted when full).
    pub fn push_committed(&mut self, raw: CharVec<24>, rendered: OutBuffer) {
        if self.edit_history_len == self.edit_history.len() {
            self.edit_history[self.edit_history_start] = None;
            self.edit_history_start = (self.edit_history_start + 1) % self.edit_history.len();
            self.edit_history_len -= 1;
        }
        let idx = (self.edit_history_start + self.edit_history_len) % self.edit_history.len();
        self.edit_history[idx] = Some(CommittedWord { raw, rendered });
        self.edit_history_len += 1;
    }

    /// Pop and return the newest committed word, or None if the ring is empty.
    pub fn pop_newest_committed(&mut self) -> Option<CommittedWord> {
        if self.edit_history_len == 0 {
            return None;
        }
        let idx = (self.edit_history_start + self.edit_history_len - 1) % self.edit_history.len();
        self.edit_history_len -= 1;
        self.edit_history[idx].take()
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
