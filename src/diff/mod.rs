//! Diff-based input API and V-C-V syllable splitting.
//!
//! The diff engine wraps the core composing engine and computes minimal
//! (backspace_count, suffix_to_type) instructions for each keystroke.

mod core;
mod state;
mod utils;

pub use state::{ComposingSnapshot, ContextHint, DiffState, EnglishFallback};

use crate::buffers::{CharVec, OutBuffer, StackStr};
use crate::engine::UltraFastViEngine;
use crate::modes::InputMethod;
use state::CommittedWord;

/// Where a host caret offset (`caret_back`) lands in the committed-word ring.
enum CaretHit {
    /// Exactly at the end of ring entry `step` (0 = newest).
    WordEnd(usize),
    /// Strictly inside ring entry `step`, `chars_before` rendered chars left
    /// of the caret.
    MidWord(usize, usize),
}

/// How the previous committed word reads on screen, for the ambiguous-English
/// rule: "English" and "Neutral" let an English word through, "Vietnamese"
/// keeps the sentence Vietnamese.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PrevWordKind {
    English,
    Neutral,
    Vietnamese,
}

/// The sentence language the commit rules judge a word by: the host's
/// `ContextHint`, or the sentence itself once a Vietnamese word is on screen.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ContextMode {
    /// Today's rules: the previous word and the rarity scores decide.
    Auto,
    /// The sentence is Vietnamese: ambiguous words keep their Vietnamese
    /// reading unless it is not a common Vietnamese syllable at all.
    Vietnamese,
    /// The sentence is English: every ambiguous word restores, whatever its
    /// Vietnamese reading scores.
    English,
}

/// Separator chars that only appear inside code: a word glued right after one
/// of them (`feed_diff`, `x=1`, `a\b`) is an identifier, not Vietnamese. A
/// `.` counts too, but only when it is glued as well (`a.b`, not "test. Next").
/// Brackets, `#`, `<>` and `|` are left out: Vietnamese text uses them
/// (`[học]`, `#việtnam`, `<ghi chú>`).
const CODE_GLUE_CHARS: [char; 7] = ['_', '`', '=', '\\', '@', '$', '.'];

/// Separator chars that END a code identifier: the word before one of them
/// shows its literal keystrokes (`raw_chars`, `feed_diff`, `UVIE_RS_DIR`).
/// `(` `/` `-` `:` `,` and a terminating `.` are normal Vietnamese
/// punctuation and are deliberately absent.
const CODE_END_CHARS: [char; 5] = ['_', '`', '=', '\\', '@'];

/// The separator char starts a code identifier when it is glued to the text
/// on its left (`foo_bar`, `a.b`).
pub(crate) fn is_code_glue_char(ch: char) -> bool {
    CODE_GLUE_CHARS.contains(&ch)
}

/// The separator char terminates a code identifier (`raw_chars`).
fn is_code_end_char(ch: char) -> bool {
    CODE_END_CHARS.contains(&ch)
}

/// Contraction suffixes that may follow a glued apostrophe ("didn't",
/// "it's", "I'm", "we'll", "they're", "I've", "I'd").
const CONTRACTION_SUFFIXES: [&str; 7] = ["t", "s", "m", "ll", "re", "ve", "d"];

/// English inflection suffixes a doubled-Telex-key stem may carry
/// ("diff" + "ed" / "s" / "ing" / "er", "off" + "s").
const DOUBLED_KEY_SUFFIXES: [&str; 7] = ["s", "es", "ed", "ing", "er", "ers", "ly"];

/// Two-letter English words a double-press tone cancel produces ("iss" →
/// "is", "ass" → "as", "orr" → "or", "iff" → "if", "uss" → "us").
const TWO_LETTER_CANCEL_WORDS: [&str; 5] = ["as", "if", "is", "or", "us"];

/// Case-insensitive `str` equality (`"Bánh"` == `"bánh"`).
fn str_eq_ignore_case(a: &str, b: &str) -> bool {
    a.chars().count() == b.chars().count()
        && a.chars()
            .zip(b.chars())
            .all(|(x, y)| x.to_lowercase().eq(y.to_lowercase()))
}

/// A Vietnamese vowel, toned or not ("Ỉ", "É", "a").
fn is_vowel_char(c: char) -> bool {
    let lower = c.to_lowercase().next().unwrap_or(c);
    VOWEL_FORMS.iter().any(|&(form, _, _)| {
        (0..=5u8).any(|tone| crate::tone::map_vowel_with_tone(form, tone) == lower)
    })
}

/// Number of vowel groups in `s` — 1 for a single syllable's nucleus.
fn vowel_runs(s: &str) -> usize {
    let mut runs = 0usize;
    let mut in_run = false;
    for c in s.chars() {
        if is_vowel_char(c) {
            if !in_run {
                runs += 1;
                in_run = true;
            }
        } else {
            in_run = false;
        }
    }
    runs
}

/// Vietnamese modifier a base letter can carry (canonical keystroke forms).
const MOD_NONE: u8 = 0;
const MOD_CIRCUMFLEX: u8 = 1;
const MOD_BREVE: u8 = 2;
const MOD_HORN: u8 = 3;
const MOD_STROKE: u8 = 4;

/// (accented vowel, plain base letter, modifier) for every Vietnamese vowel
/// form the engine renders — precomposed NFC, without a tone.
const VOWEL_FORMS: [(char, char, u8); 12] = [
    ('a', 'a', MOD_NONE),
    ('ă', 'a', MOD_BREVE),
    ('â', 'a', MOD_CIRCUMFLEX),
    ('e', 'e', MOD_NONE),
    ('ê', 'e', MOD_CIRCUMFLEX),
    ('i', 'i', MOD_NONE),
    ('o', 'o', MOD_NONE),
    ('ô', 'o', MOD_CIRCUMFLEX),
    ('ơ', 'o', MOD_HORN),
    ('u', 'u', MOD_NONE),
    ('ư', 'u', MOD_HORN),
    ('y', 'y', MOD_NONE),
];

/// Tone key for tone `tone` (1 = sắc … 5 = nặng) in `method`.
fn canonical_tone_key(method: InputMethod, tone: u8) -> char {
    match method {
        InputMethod::Vni => (b'0' + tone) as char,
        _ => match tone {
            1 => 's',
            2 => 'f',
            3 => 'r',
            4 => 'x',
            _ => 'j',
        },
    }
}

/// Keys that turn `base` into the letter carrying `modifier` (`upper` picks
/// the uppercase form of every letter key; tone keys and single modifiers
/// stay lowercase).
fn push_canonical_modifier(
    method: InputMethod,
    base: char,
    modifier: u8,
    upper: bool,
    raw: &mut CharVec<24>,
) -> bool {
    let base = if upper {
        base.to_ascii_uppercase()
    } else {
        base
    };
    let modifier_key = match method {
        InputMethod::Vni => match modifier {
            MOD_CIRCUMFLEX => '6',
            MOD_BREVE => '8',
            MOD_HORN => '7',
            MOD_STROKE => '9',
            _ => return raw.try_push(base),
        },
        // Telex doubles the letter for the circumflex and for `đ`; `w` adds
        // the breve/horn to the base letter.
        _ => match modifier {
            MOD_CIRCUMFLEX | MOD_STROKE => return raw.try_push(base) && raw.try_push(base),
            MOD_BREVE | MOD_HORN => 'w',
            _ => return raw.try_push(base),
        },
    };
    raw.try_push(base) && raw.try_push(modifier_key)
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
            let (bs, _) = self.commit_with_boundary_diff(ch);
            self.diff.note_separator(ch);
            let _ = self.diff.diff_suffix.push(ch);
            return (bs, &self.diff.diff_suffix);
        }

        // The word outgrew the 24-keystroke raw buffer: it cannot be one
        // Vietnamese syllable, and `word_raw` no longer describes the whole
        // token, so no restore rule may rebuild it. Show the literal
        // keystrokes typed so far and pass the rest of the word through raw
        // (the sticky path) — a long identifier must never lose a key.
        if self.diff.word_raw.is_full() && !self.diff.english_sticky {
            let (bs, _) = self.begin_raw_passthrough();
            let _ = self.diff.prev_rendered.push(ch);
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
            word_had_cancel: self.diff.word_had_cancel,
            word_had_modifier_cancel: self.diff.word_had_modifier_cancel,
        });

        // Append to key_log and word_raw, then run the core pipeline.
        let _ = self.diff.key_log.try_push(ch);
        let _ = self.diff.word_raw.try_push(ch);
        // First keystroke of the word: remember whether it started right
        // after a code char glued to the text before it (`feed_diff`,
        // `a.b`) — the whole token is an identifier then, not Vietnamese —
        // or inside a URL / path token (`/usr/share`, `github.com/x`).
        if self.diff.word_raw.len() == 1 {
            self.diff.resolve_sentence_break(false);
            self.diff.word_after_code =
                self.diff.last_sep_glued && is_code_glue_char(self.diff.last_sep);
            self.diff.word_in_url = self.diff.token_url;
        }
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
        // The dictionary reads the word the user MEANT: a modifier-key cancel
        // ("awwa" → "awa") leaves no trace in the render, so it must leave
        // none in the spelling either. A doubled tone key is genuine English
        // spelling and keeps every key ("mass" is looked up as "mass").
        let dict_word = if !dict_eligible || !self.enable_english_override {
            None
        } else if self.diff.word_had_modifier_cancel {
            let effective = self.effective_word_raw();
            crate::tables::is_english_override(&effective).then_some(effective)
        } else if crate::tables::is_english_override(&self.diff.word_raw) {
            Some(self.diff.word_raw.clone())
        } else {
            None
        };
        if let Some(dict_word) = dict_word {
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
            for &c in dict_word.iter() {
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
                        word_had_cancel: self.diff.word_had_cancel,
                        word_had_modifier_cancel: self.diff.word_had_modifier_cancel,
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
            self.diff.word_had_cancel = snap.word_had_cancel;
            self.diff.word_had_modifier_cancel = snap.word_had_modifier_cancel;

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
        self.commit_with_boundary_diff('\0')
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
        // A word whose raw no longer types its rendered text (a native
        // Backspace on English input) cannot be re-rendered: decline.
        match self.resolve_caret(caret_back)? {
            CaretHit::WordEnd(step) => {
                if !self.diff.committed(step)?.editable {
                    return None;
                }
                self.edit_word_end(step, caret_back, ch)
            }
            CaretHit::MidWord(step, chars_before) => {
                if !self.diff.committed(step)?.editable {
                    return None;
                }
                self.edit_mid_word(step, chars_before, ch)
            }
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
        // The separator history describes text the engine no longer tracks.
        self.diff.clear_separator();
        // So does the sentence context (a host caret jump starts a new one).
        self.diff.clear_sentence();
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
    /// Commit the composing word at `boundary` — the separator key that ended
    /// it (`' '`, `'.'`, `'_'`, `'\n'`, ...; `'\0'` when the host has no char).
    /// Returns the minimal edit (backspaces, suffix) that replaces the
    /// Vietnamese transform with the word the user meant.
    ///
    /// The rules, in order (all gated by `set_english_override`):
    /// 1. dictionary — a known English word;
    /// 2. code identifier / doubled tone key / ALL-CAPS acronym — the raw
    ///    keystrokes are decisive whatever the context;
    /// 3. restore-raw — the render cannot be a Vietnamese word;
    /// 4. ambiguous English word — the Vietnamese rarity score plus the
    ///    previous word decide ("how" after "Hello" is English, "hơ" after
    ///    "các" is Vietnamese).
    ///
    /// Whenever the word ends up English, older ambiguous words separated by
    /// a single char are restored too ("this is a test" must not stay
    /// "thí í a tét"), and the returned plan rewrites that whole span. An
    /// ALL-CAPS acronym or a code identifier is not English proof and restores
    /// nothing in front of it ("car CI/CD" stays "cả CI/CD").
    ///
    /// A double-press cancel commits the cancelled render ("carr" → "car")
    /// unless the doubled key spells a genuinely English word ("off", "err").
    pub fn commit_with_boundary_diff(&mut self, boundary: char) -> (usize, &str) {
        // Nothing composing: there is no word to record. An idle commit (a
        // boundary key pressed after punctuation or a second Space) must not
        // push an empty "phantom" entry — the host measures its caret offset
        // from the real word, so a phantom would shift the geometry by one.
        if self.diff.word_raw.is_empty() && !self.is_composing_diff() {
            self.diff.diff_suffix.clear();
            return (0, &self.diff.diff_suffix);
        }

        let gate = self.enable_english_override && !self.diff.word_raw.is_empty();
        // The sentence language this word is judged by: the host's hint, or
        // the sentence itself once a Vietnamese word is on screen.
        let ctx = self.context_mode();
        // The word as the user meant it: a modifier-key cancel leaves no trace
        // in the render, so it leaves none in the spelling either ("awware" →
        // "aware"). Every rebuild from the raw keystrokes uses this.
        let effective = if gate {
            self.effective_word_raw()
        } else {
            self.diff.word_raw.clone()
        };

        // An ALL-CAPS acronym or a code identifier is no proof that the
        // sentence is English, so such a word must not restore the words in
        // front of it ("car CI/CD" stays "cả CI/CD").
        let retro_ok = gate && self.retro_allowed(boundary);

        // English dictionary override: if the full word matches a known
        // English word, replace the Vietnamese transform with the raw English
        // word. The diff engine computes the backspaces needed to transform
        // what's on screen (diff_committed + prev_rendered) into the raw word.
        // In a Vietnamese sentence a render that is a common Vietnamese
        // syllable is what the user meant, so the dictionary leaves it alone.
        if gate
            && crate::tables::is_english_override(&effective)
            && !Self::keeps_vietnamese_render(ctx, &self.screen_text())
        {
            return self.commit_restoring_raw(retro_ok);
        }

        // Contraction: the newest word is followed by exactly one glued
        // apostrophe and this word is its suffix ("didn't", "it's", "we'll").
        // The word before the apostrophe is English too, so it must show its
        // literal keystrokes — the span rewrite covers both words.
        if gate && self.is_contraction() {
            return self.commit_contraction();
        }

        // Raw-keystroke rules: a code identifier, an English word whose
        // doubled Telex tone key the cancel sequence collapsed ("off" →
        // "of"), an ALL-CAPS acronym the pipeline read as a rare syllable
        // ("JIRA" → "JỈA"). None of them depends on the sentence context.
        // A word that outgrew the raw buffer only knows its tail, so the
        // restore would rewrite text the raw no longer covers: skip it.
        if gate
            && !self.diff.word_raw_overflowed
            && self.should_restore_code_or_english_token(boundary)
        {
            return self.commit_restoring_raw(retro_ok);
        }

        // Hyphen compound: one side of a '-' glued pair being English makes
        // the other side English too ("low-level", "backspace-arms").
        if gate && !self.diff.word_raw_overflowed && self.hyphen_english_neighbor() {
            return self.commit_restoring_raw(retro_ok);
        }

        // Restore-raw rule (LabanKey): a finished token that is not a valid
        // Vietnamese word shows the literal keystrokes instead of the marks
        // the pipeline applied to it ("law" → "lă", "after" → "àtẻ"). Same
        // toggle as the dictionary — the host's "restore English" setting.
        // A double-press cancel left a plain-ASCII render ("forr" → "for",
        // "hoss" → "hos"): that render is what the user asked for, so the
        // context rules below must not bring the cancelled key back.
        let cancel_kept = gate && self.cancel_left_ascii_render();

        if gate
            && !cancel_kept
            && !self.diff.word_raw_overflowed
            && self.should_restore_raw_on_commit()
        {
            return self.commit_restoring_raw(retro_ok);
        }

        // Ambiguous English word: a common English word that renders as a
        // valid Vietnamese syllable in isolation ("how" → "hơ", "best" →
        // "bét") is restored when its Vietnamese reading is rare or the
        // previous committed word is English. Vietnamese context never
        // qualifies, so Vietnamese typing is unaffected.
        if gate
            && !cancel_kept
            && !self.diff.word_raw_overflowed
            && self.should_restore_ambiguous_english(ctx)
        {
            return self.commit_restoring_raw(retro_ok);
        }

        // Capture the word for post-commit editing before the clears wipe
        // the buffers: raw keystrokes + the exact text on screen (V-C-V
        // committed prefix + composing render).
        let entry_raw = effective;
        let mut entry_rendered = crate::buffers::new_out_buffer();
        let _ = entry_rendered.push_str(&self.diff.diff_committed);
        let _ = entry_rendered.push_str(&self.diff.prev_rendered);
        // A plainly-typed English word needs no restore, but it still proves
        // the sentence is English: older ambiguous words may be restored. A
        // word that outgrew the raw buffer only knows its tail — skip it.
        // A word whose cancel collapsed to a known English word ("carr" →
        // "car") proves it just the same.
        let plain_english = retro_ok
            && !self.diff.word_raw_overflowed
            && (self.cancel_render_is_english_word()
                || self.word_is_plain_english(&entry_rendered)
                || self.hyphen_compound_is_english(&entry_rendered));
        let entry_had_cancel = self.diff.word_had_cancel;
        // The raw keystrokes no longer describe the whole word: the entry
        // keeps its text and geometry but declines post-commit edits.
        let entry_overflowed = self.diff.word_raw_overflowed;

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
        self.diff.word_had_cancel = false;
        self.diff.word_had_modifier_cancel = false;
        self.diff.english_sticky = false;
        self.diff.english_fallback = None;
        // Clear the snapshot stack — stale snapshots from the committed word
        // must not survive, otherwise a backspace after commit would restore
        // state from the previous word, corrupting the engine.
        for s in &mut self.diff.snapshots {
            *s = None;
        }
        self.diff.snapshot_count = 0;
        // The word stays on screen as typed: only the older words can need a
        // rewrite, and the ring must not contain the current word yet.
        let retro = if plain_english {
            self.retro_restore_english(ctx, &entry_rendered, &entry_rendered)
        } else {
            None
        };
        // Record the word now that the clears are done (push_committed must
        // not be wiped by the diff-state clear above).
        self.diff.push_committed(entry_raw, entry_rendered);
        if entry_had_cancel {
            if let Some(entry) = self.diff.committed_mut(0) {
                entry.had_cancel = true;
            }
        }
        if entry_overflowed {
            if let Some(entry) = self.diff.committed_mut(0) {
                entry.editable = false;
            }
        }
        (retro.unwrap_or(0), &self.diff.diff_suffix)
    }

    /// A separator char (space / punctuation / Enter) was typed right after
    /// the newest committed word: grow its on-screen gap. No-op while nothing
    /// is committed.
    ///
    /// The char itself matters: a code char glued to the text before it
    /// (`_` in `foo_bar`, `.` in `a.b`) makes the next word a code
    /// identifier, and the retroactive English fix needs the exact separator
    /// between two words.
    pub fn note_separator(&mut self, ch: char) {
        self.diff.note_separator(ch);
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

    /// A word is on record (the caret offsets are measured against it).
    pub fn has_committed_word(&self) -> bool {
        self.diff.edit_history_len > 0
    }

    /// Set the sentence-language hint the commit rules use (a host that runs
    /// a language recognizer over `sentence_text()`).
    ///
    /// The hint describes the sentence being typed, so it resets to `Auto` at
    /// a sentence boundary (`!`, `?`, a newline, or a `.` that ends a word)
    /// and on every reset — a stale verdict must never outlive its sentence.
    pub fn set_context_hint(&mut self, hint: ContextHint) {
        self.diff.context_hint = hint;
    }

    /// The current sentence-language hint.
    pub fn context_hint(&self) -> ContextHint {
        self.diff.context_hint
    }

    /// The current sentence so far: its committed words with their
    /// separators, oldest first — the text a host runs its language
    /// recognizer on.
    ///
    /// Rebuilt from the committed-word ring, so a sentence longer than the
    /// ring keeps only its newest words; it never includes the composing
    /// word. Ambiguous English words ("cos"/"có", "this"/"thí") are left out:
    /// whichever reading is on screen would bias the recognizer.
    pub fn sentence_text(&self) -> StackStr<512> {
        self.diff.sentence_text()
    }

    /// Caret offset (`caret_back`, screen chars from the end of the newest
    /// committed word) the host must adopt after the last successful edit.
    /// Negative means the caret is right of the new newest word's end.
    pub fn last_caret_back_after_edit(&self) -> isize {
        self.diff.last_edit_caret_back
    }

    /// The host deleted one visible char natively: Backspace while idle with
    /// the caret exactly at a committed word's end. The OS owns the visible
    /// edit (one char, one key press — a held key repeats cleanly, unlike the
    /// synthetic delete+retype pairs the old keystroke-undo did); the engine
    /// only tracks it so the word stays editable: its rendered text shrinks
    /// by one char and its raw keystrokes are rebuilt from the rest.
    ///
    /// Returns true when the engine tracked the deletion. The host passes the
    /// key through either way; on false it also drops its caret history.
    /// Mid-word carets and an empty ring decline.
    pub fn delete_char_at_caret(&mut self, caret_back: usize) -> bool {
        if self.is_composing_diff() {
            return false;
        }
        let Some(CaretHit::WordEnd(step)) = self.resolve_caret(caret_back) else {
            return false;
        };
        let Some(entry) = self.diff.committed(step) else {
            return false;
        };
        let mut new_rendered = entry.rendered.clone();
        if new_rendered.pop().is_none() {
            return false;
        }

        if new_rendered.is_empty() {
            // The word is gone: drop the entry and let the older neighbor
            // absorb its separators (they become adjacent on screen).
            let older_gap = self.diff.committed(step + 1).map(|w| w.gap);
            let Some(removed) = self.diff.remove_committed(step) else {
                return false;
            };
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
            return true;
        }

        // Rebuild the keystrokes that type the remaining text. The word stays
        // editable only when they render back to it exactly; an English word
        // whose letters the engine would transform when re-typed ("wa": a
        // leading `w` is the ư modifier) keeps its geometry but declines edits.
        let new_raw = self.canonical_raw(&new_rendered);
        let prev = self.diff.committed(step + 1).cloned();
        let editable = new_raw
            .as_ref()
            .is_some_and(|raw| self.render_committed_word(raw, prev.as_ref()) == new_rendered);
        let new_raw = new_raw.filter(|_| editable);
        let Some(entry) = self.diff.committed_mut(step) else {
            return false;
        };
        entry.rendered = new_rendered;
        entry.editable = new_raw.is_some();
        if let Some(raw) = new_raw {
            entry.raw = raw;
        }
        // The deleted char sat LEFT of the caret, so the caret and the anchor
        // both shift left by one: the caret-to-anchor distance is unchanged.
        self.diff.last_edit_caret_back = caret_back as isize;
        true
    }

    /// Keystrokes that type `rendered` in the current input method (Telex:
    /// â=aa ă=aw ê=ee ô=oo ơ=ow ư=uw đ=dd, tones s f r x j; VNI: a6 e6 o6 a8
    /// o7 u7 d9, tones 1..5), with every tone key posted at the END of the
    /// word — the natural Telex/VNI typing order. Case is preserved: an
    /// uppercase base letter takes uppercase keys and doubled letters follow
    /// it ("Ê" = "EE"), while tone keys and single modifiers stay lowercase.
    ///
    /// Returns None when a char has no keystroke encoding or the raw buffer
    /// overflows; the caller then falls back to a non-editable entry.
    fn canonical_raw(&self, rendered: &str) -> Option<CharVec<24>> {
        let mut raw = CharVec::<24>::new();
        let mut tones = CharVec::<8>::new();
        for c in rendered.chars() {
            if !self.push_canonical_char(c, &mut raw, &mut tones) {
                return None;
            }
        }
        for &tone in tones.iter() {
            if !raw.try_push(tone) {
                return None;
            }
        }
        Some(raw)
    }

    /// One rendered char's canonical keystrokes (`raw`), with its tone key
    /// collected into `tones` for the end-of-word placement.
    fn push_canonical_char(&self, c: char, raw: &mut CharVec<24>, tones: &mut CharVec<8>) -> bool {
        if c.is_ascii() {
            return raw.try_push(c);
        }
        let lower = c.to_lowercase().next().unwrap_or(c);
        let upper = c.is_uppercase();
        if lower == 'đ' {
            return push_canonical_modifier(self.input_method, 'd', MOD_STROKE, upper, raw);
        }
        // Accented vowel: recover the plain base letter, the modifier and the
        // tone by scanning the engine's own tone table.
        for &(form, base, modifier) in VOWEL_FORMS.iter() {
            for tone in 0..=5u8 {
                if crate::tone::map_vowel_with_tone(form, tone) != lower {
                    continue;
                }
                if tone > 0 && !tones.try_push(canonical_tone_key(self.input_method, tone)) {
                    return false;
                }
                return push_canonical_modifier(self.input_method, base, modifier, upper, raw);
            }
        }
        false
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
    /// every restore rule.
    ///
    /// The word is English, so the ambiguous words in front of it are too:
    /// the retroactive fix rewrites the span from the oldest restored word
    /// (`retro` is false when the word cannot prove the sentence language —
    /// an ALL-CAPS acronym or a code identifier).
    fn commit_restoring_raw(&mut self, retro: bool) -> (usize, &str) {
        // Build full on-screen text (Vietnamese) in a stack buffer.
        let mut full_screen = crate::buffers::new_out_buffer();
        let _ = full_screen.push_str(&self.diff.diff_committed);
        let _ = full_screen.push_str(&self.diff.prev_rendered);

        // The spelling the user meant: a modifier-key cancel leaves no trace
        // ("awware" → "aware"), a doubled tone key stays ("off", "diffed").
        let effective = self.effective_word_raw();

        // Build target text (raw keystrokes, preserve case) in a stack buffer.
        let mut target = crate::buffers::new_out_buffer();
        for &c in effective.iter() {
            let _ = target.push(c);
        }

        // Capture the raw keystrokes BEFORE the clears wipe word_raw.
        let entry_raw = effective;
        // A word that outgrew the raw buffer only knows its tail, so the
        // span the retroactive fix would rebuild is incomplete: skip it.
        let overflowed = self.diff.word_raw_overflowed;

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

        // The ring still holds the older words and not this one: restore the
        // ambiguous ones the English word proves were English too.
        let ctx = self.context_mode();
        let retro = if !retro || overflowed {
            None
        } else {
            self.retro_restore_english(ctx, &full_screen, &target)
        };

        // Record the word for post-commit editing: on screen it is the raw
        // keystrokes (the transform was replaced), so the ring entry needs no
        // cancel bookkeeping — rendered == raw already blocks a retro restore.
        self.diff.push_committed(entry_raw, target.clone());

        match retro {
            Some(bs) => (bs, &self.diff.diff_suffix),
            None => {
                // Diff from full_screen → target, writing suffix into diff_suffix.
                let (bs, _) = Self::diff_into(&full_screen, &target, &mut self.diff.diff_suffix);
                (bs, &self.diff.diff_suffix)
            }
        }
    }

    /// Commit a contraction ("didn't", "it's", "we'll"): the word before the
    /// glued apostrophe shows its literal keystrokes and the current word
    /// commits as typed. The plan rewrites the whole span (previous word +
    /// apostrophe + this word); the ring entry of the previous word is
    /// updated in place so it stays editable.
    fn commit_contraction(&mut self) -> (usize, &str) {
        let mut full_screen = crate::buffers::new_out_buffer();
        let mut target = crate::buffers::new_out_buffer();
        if let Some(prev) = self.diff.committed(0) {
            let _ = full_screen.push_str(&prev.rendered);
            let _ = full_screen.push('\'');
            for &c in prev.raw.iter() {
                let _ = target.push(c);
            }
            let _ = target.push('\'');
        }
        let _ = full_screen.push_str(&self.diff.diff_committed);
        let _ = full_screen.push_str(&self.diff.prev_rendered);
        for &c in self.diff.word_raw.iter() {
            let _ = target.push(c);
        }

        let entry_raw = self.diff.word_raw.clone();
        let mut entry_rendered = crate::buffers::new_out_buffer();
        let _ = entry_rendered.push_str(&self.diff.diff_committed);
        let _ = entry_rendered.push_str(&self.diff.prev_rendered);

        // Clear the composing state, keeping the ring: the previous word is
        // still on screen (now as typed) and stays editable.
        self.buf.clear();
        self.raw_len = 0;
        self.pending_tone = None;
        self.out_buf.clear();
        self.committed.clear();
        self.syl_structure.clear();
        let ring = ::core::mem::take(&mut self.diff.edit_history);
        let ring_start = self.diff.edit_history_start;
        let ring_len = self.diff.edit_history_len;
        self.diff.clear();
        self.diff.edit_history = ring;
        self.diff.edit_history_start = ring_start;
        self.diff.edit_history_len = ring_len;

        if let Some(prev) = self.diff.committed_mut(0) {
            let mut rendered = crate::buffers::new_out_buffer();
            for &c in prev.raw.iter() {
                let _ = rendered.push(c);
            }
            prev.rendered = rendered;
        }
        let (bs, _) = Self::diff_into(&full_screen, &target, &mut self.diff.diff_suffix);
        self.diff.push_committed(entry_raw, entry_rendered);
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
        // The spelling the user meant: a modifier-key cancel leaves no trace,
        // so a render that already matches it needs no restore ("awware").
        for &c in self.effective_word_raw().iter() {
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
        // A V-C-V split inside this token glued several syllables together,
        // which a single Vietnamese word never is.
        if !self.diff.diff_committed.is_empty() {
            return true;
        }
        // Plain-letter differences come from intentional double-key cancels
        // ("masster" → "master", "off" → "of") — those must stay. The one
        // exception is a cancelled tone key between two `a` halves: that pair
        // was a failed `aa` → `â`, and an open `â` syllable is never a
        // Vietnamese word, so the literal keystrokes win ("rassa", "taxxa").
        if screen.is_ascii() {
            if Self::cancelled_a_circumflex_pair(&self.diff.word_raw, self.mode) {
                return true;
            }
            // A modifier key the pipeline swallowed after the last visible
            // letter ("browserclaw" → "browsercla") leaves the render as a
            // prefix of the keystrokes. One Vietnamese syllable has a single
            // vowel group, so a multi-group ASCII render is an English word.
            let mut raw_iter = raw.chars();
            let is_prefix =
                screen.chars().all(|c| raw_iter.next() == Some(c)) && raw_iter.next().is_some();
            return is_prefix && vowel_runs(&screen) > 1;
        }
        crate::restore::rendered_syllable_needs_restore(&self.diff.prev_rendered)
    }

    /// Raw-keystroke rules — decisive whatever the sentence context is:
    ///
    /// - a code identifier: the word starts right after a code char glued to
    ///   the text before it (`feed_diff`, `a.b`) or the boundary char ends it
    ///   (`raw_chars`). `(` `/` `-` `:` `,` and a terminating `.` are normal
    ///   Vietnamese punctuation and never trigger;
    /// - an English word with a doubled Telex tone key ("off", "diff",
    ///   "lossless"): the second key is the cancel sequence, so the render
    ///   lost a letter;
    /// - an ALL-CAPS acronym whose render is a rare Vietnamese syllable
    ///   ("JIRA" → "JỈA").
    fn should_restore_code_or_english_token(&self, boundary: char) -> bool {
        if self.diff.word_after_code || self.diff.word_in_url || is_code_end_char(boundary) {
            return true;
        }
        // A camelCase / mixed-case token is an identifier, never Vietnamese
        // typing (which only capitalizes the first letter).
        if Self::is_mixed_case_token(&self.diff.word_raw) {
            return true;
        }
        // No Vietnamese syllable starts with j, f or z: a render that does
        // was transformed from an English word ("Jira" → "Jỉa"). Quick Start
        // expands j to gi before the render, so "jira" stays "gira" there.
        if self.render_starts_with_foreign_onset() {
            return true;
        }
        if self.is_always_english_tech_word() {
            return true;
        }
        if Self::is_english_suffixed_doubled_key(&self.diff.word_raw) {
            return true;
        }
        if crate::tables::is_english_doubled_key_word(&self.diff.word_raw) {
            return !self.cancel_render_is_english_word();
        }
        self.should_restore_acronym()
    }

    /// A token with a lowercase letter followed later by an uppercase one is
    /// a camelCase identifier ("DeepSeek", "applyCompoundForwardDeletes").
    fn is_mixed_case_token(raw: &[char]) -> bool {
        let mut seen_lower = false;
        for &c in raw {
            if c.is_ascii_lowercase() {
                seen_lower = true;
            } else if seen_lower && c.is_ascii_uppercase() {
                return true;
            }
        }
        false
    }

    /// An English word built from a doubled-Telex-key stem plus an inflection
    /// ("diff" + "ed"/"s"/"ing"/"er", "off" + "s"): the cancel collapsed a
    /// letter, but the raw keystrokes are real English and must show as typed.
    fn is_english_suffixed_doubled_key(raw: &[char]) -> bool {
        for suffix in DOUBLED_KEY_SUFFIXES {
            let n = suffix.len();
            if raw.len() <= n {
                continue;
            }
            let stem = &raw[..raw.len() - n];
            let tail = &raw[raw.len() - n..];
            let matches = tail
                .iter()
                .zip(suffix.chars())
                .all(|(&a, b)| a.eq_ignore_ascii_case(&b));
            if matches && crate::tables::is_english_doubled_key_word(stem) {
                return true;
            }
        }
        false
    }

    /// The render starts with j, f or z (any case) and differs from the
    /// literal keystrokes: no Vietnamese syllable has that onset, so the
    /// pipeline transformed an English word.
    fn render_starts_with_foreign_onset(&self) -> bool {
        let first = self
            .diff
            .diff_committed
            .chars()
            .next()
            .or_else(|| self.diff.prev_rendered.chars().next());
        let foreign = matches!(
            first.map(|c| c.to_ascii_lowercase()),
            Some('j') | Some('f') | Some('z')
        );
        foreign && self.screen_differs_from_raw()
    }

    /// The word's render differs from its literal keystrokes.
    fn screen_differs_from_raw(&self) -> bool {
        let mut raw = self.diff.word_raw.iter().copied();
        let mut screen = self
            .diff
            .diff_committed
            .chars()
            .chain(self.diff.prev_rendered.chars());
        loop {
            match (raw.next(), screen.next()) {
                (None, None) => return false,
                (Some(a), Some(b)) if a == b => {}
                _ => return true,
            }
        }
    }

    /// An always-English tech word in lowercase, except right after the
    /// Vietnamese "bánh" ("bánh tét" is a dish, not the English "test").
    fn is_always_english_tech_word(&self) -> bool {
        if !crate::tables::is_always_english_tech_word(&self.diff.word_raw) {
            return false;
        }
        if !self.diff.word_raw.iter().all(|c| c.is_ascii_lowercase()) {
            return false;
        }
        !self
            .diff
            .committed(0)
            .is_some_and(|prev| str_eq_ignore_case(&prev.rendered, "bánh"))
    }

    /// The newest committed word is glued to this one by a single '-' and
    /// reads as English ("low-level", "backspace-arms"): the compound's other
    /// side is English too.
    fn hyphen_english_neighbor(&self) -> bool {
        let Some(prev) = self.diff.committed(0) else {
            return false;
        };
        if prev.gap != 1 || prev.sep != '-' {
            return false;
        }
        if self.prev_word_kind(prev) == PrevWordKind::English {
            return true;
        }
        // A word typed as plain ASCII that is no Vietnamese syllable is
        // English in a hyphen compound even when it is not in any table
        // ("backspace").
        prev.rendered.chars().all(|c| c.is_ascii_alphabetic())
            && !crate::tables::is_common_vietnamese_syllable(&prev.rendered)
            && !crate::tables::is_top_vietnamese_syllable(&prev.rendered)
            && !crate::tables::is_vietnamese_loanword(&prev.rendered)
    }

    /// The current word is plain ASCII that is no Vietnamese syllable, and
    /// the newest committed word is glued to it by a single '-': the hyphen
    /// compound is English ("low-level"), so the Vietnamese-looking side in
    /// front of it may be restored.
    fn hyphen_compound_is_english(&self, screen: &str) -> bool {
        let Some(prev) = self.diff.committed(0) else {
            return false;
        };
        if prev.gap != 1 || prev.sep != '-' {
            return false;
        }
        screen.chars().all(|c| c.is_ascii_alphabetic())
            && !crate::tables::is_common_vietnamese_syllable(screen)
            && !crate::tables::is_top_vietnamese_syllable(screen)
    }

    /// A contraction: the newest committed word is followed by exactly one
    /// glued apostrophe and the current word is a contraction suffix
    /// ("didn't", "it's", "I'm", "we'll", "they're").
    fn is_contraction(&self) -> bool {
        let Some(prev) = self.diff.committed(0) else {
            return false;
        };
        if prev.gap != 1 || prev.sep != '\'' {
            return false;
        }
        let raw = &self.diff.word_raw;
        CONTRACTION_SUFFIXES.iter().any(|suffix| {
            raw.len() == suffix.chars().count()
                && raw
                    .iter()
                    .zip(suffix.chars())
                    .all(|(&a, b)| a.eq_ignore_ascii_case(&b))
        })
    }

    /// A double-press cancel leaves the cancelled word on screen ("carr"
    /// renders "car"), and that render is what the user asked for. When it is
    /// itself a known English word of at least three letters, the
    /// doubled-key English rule must not re-insert the cancelling key.
    ///
    /// A two-letter cancelled form ("of", "er") is too weak a signal: the
    /// doubled key is far more likely part of a genuinely English word, so
    /// "off" and "err" keep their keys.
    fn cancel_render_is_english_word(&self) -> bool {
        if !self.diff.word_had_cancel {
            return false;
        }
        // The whole word on screen (V-C-V prefix + composing render).
        let mut screen = crate::buffers::new_out_buffer();
        let _ = screen.push_str(&self.diff.diff_committed);
        let _ = screen.push_str(&self.diff.prev_rendered);
        if !screen.chars().all(|c| c.is_ascii_alphabetic()) {
            return false;
        }
        // Two-letter English words whose Telex keys get a tone ("is" → "í",
        // "as" → "á"): the double press is the only way to type them, so the
        // cancelled form wins over the rarer doubled spelling ("iss" → "is").
        // "off" stays: it is far more common than "of" typed that way.
        if screen.chars().count() == 2 {
            let lower = screen.to_ascii_lowercase();
            return TWO_LETTER_CANCEL_WORDS.contains(&lower.as_str());
        }
        if screen.chars().count() < 3 {
            return false;
        }
        let mut chars = CharVec::<24>::new();
        for c in screen.chars() {
            if !chars.try_push(c) {
                return false;
            }
        }
        crate::tables::is_english_context_word(&screen)
            || crate::tables::is_english_override(&chars)
            || crate::tables::is_english_ambiguous_word(&chars)
    }

    /// A double-press cancel fired and the word on screen is plain ASCII
    /// letters: the cancelled render, which must stay as it is.
    fn cancel_left_ascii_render(&self) -> bool {
        self.diff.word_had_cancel
            && !self.diff.prev_rendered.is_empty()
            && self
                .diff
                .diff_committed
                .chars()
                .chain(self.diff.prev_rendered.chars())
                .all(|c| c.is_ascii_alphabetic())
    }

    /// The word as it is on screen right now (V-C-V committed prefix + the
    /// composing render).
    fn screen_text(&self) -> OutBuffer {
        let mut screen = crate::buffers::new_out_buffer();
        let _ = screen.push_str(&self.diff.diff_committed);
        let _ = screen.push_str(&self.diff.prev_rendered);
        screen
    }

    /// The word's keystrokes with every modifier-key cancel's consumed key
    /// removed — the spelling of the word the user meant ("awware" →
    /// "aware", "newws" → "news", "aww" → "aw").
    ///
    /// Only a doubled MODIFIER collapses: `w`, a doubled `a`/`e`/`o` (the
    /// circumflex triple-cancel) and `dd` are never part of an English
    /// spelling, while a doubled tone key is ("off", "diffed", "mass") and
    /// keeps every key. Derived by replaying the word on a scratch engine and
    /// dropping the keys the pipeline consumed; it only runs once a modifier
    /// cancel fired, so the common per-keystroke path pays nothing.
    fn effective_word_raw(&self) -> CharVec<24> {
        if !self.diff.word_had_modifier_cancel || self.diff.word_raw_overflowed {
            return self.diff.word_raw.clone();
        }
        let mut scratch = self.scratch_engine(false);
        let mut out = CharVec::<24>::new();
        let mut prev: Option<char> = None;
        for &c in self.diff.word_raw.iter() {
            let before = scratch.raw_len();
            let _ = scratch.feed(c);
            let kept = scratch.raw_len() > before;
            // A consumed key is only a cancel when it repeats the previous key
            // — the double press that undoes the transform. A `w` the pipeline
            // swallowed for another reason ("browserclaw", "chuwongw") is not
            // a cancel: the restore-raw rule still needs that key.
            let double_press = prev.is_some_and(|p| p.eq_ignore_ascii_case(&c));
            if kept || !double_press || !Self::is_modifier_cancel_key(self.mode, c) {
                let _ = out.try_push(c);
            }
            prev = Some(c);
        }
        out
    }

    /// A keystroke that only ever cancels a transform when pressed twice: the
    /// `w` modifier, a doubled `a`/`e`/`o` (Telex circumflex), `dd`, or VNI's
    /// `6`/`7`/`8`/`9`.
    fn is_modifier_cancel_key(mode: &crate::modes::Mode, ch: char) -> bool {
        if !ch.is_ascii() {
            return false;
        }
        if mode.classify[ch as usize] & crate::modes::IS_MODIFIER != 0 {
            return true;
        }
        mode.resolver == crate::modes::ResolverKind::Telex && matches!(ch, 'a' | 'e' | 'o')
    }

    /// The sentence language the commit rules judge this word by.
    fn context_mode(&self) -> ContextMode {
        match self.diff.context_hint {
            ContextHint::Vietnamese => ContextMode::Vietnamese,
            ContextHint::English => ContextMode::English,
            ContextHint::Auto if self.diff.sentence_vi => ContextMode::Vietnamese,
            ContextHint::Auto => ContextMode::Auto,
        }
    }

    /// In a Vietnamese sentence a render that is a common or top Vietnamese
    /// syllable is what the user meant: the English rules leave it alone.
    fn keeps_vietnamese_render(ctx: ContextMode, rendered: &str) -> bool {
        ctx == ContextMode::Vietnamese
            && (crate::tables::is_top_vietnamese_syllable(rendered)
                || crate::tables::is_common_vietnamese_syllable(rendered))
    }

    /// Whether the current word may restore the ambiguous English words in
    /// front of it. An ALL-CAPS acronym ("CI", "CD", "JIRA") or a code
    /// identifier proves nothing about the sentence language — Vietnamese
    /// prose is full of them ("dùng cả CI/CD", "nó gọi API") — so it must not
    /// rewrite the Vietnamese words before it.
    fn retro_allowed(&self, boundary: char) -> bool {
        !self.diff.word_after_code
            && !is_code_end_char(boundary)
            && !Self::is_all_caps_token(&self.diff.word_raw)
    }

    /// Every letter is uppercase ASCII and there are at least two of them;
    /// digits are allowed ("CI", "CD", "API", "JIRA", "HTTP2").
    fn is_all_caps_token(raw: &[char]) -> bool {
        let mut letters = 0usize;
        for &c in raw {
            if c.is_ascii_uppercase() {
                letters += 1;
            } else if !c.is_ascii_digit() {
                return false;
            }
        }
        letters >= 2
    }

    /// ALL-CAPS acronym rule: 2..=5 uppercase ASCII letters whose render is
    /// ONE syllable the Vietnamese word list barely uses ("JIRA" → "JỈA",
    /// "DESC" → "DÉC") is English. A common syllable keeps the word
    /// Vietnamese ("VIEETJ" renders "VIỆT").
    fn should_restore_acronym(&self) -> bool {
        let raw = &self.diff.word_raw;
        if !(2..=5).contains(&raw.len()) || !raw.iter().all(|c| c.is_ascii_uppercase()) {
            return false;
        }
        // A V-C-V split means the render is two syllables, not an acronym's
        // Vietnamese reading.
        if !self.diff.diff_committed.is_empty() {
            return false;
        }
        let rendered = &self.diff.prev_rendered;
        // Already literal — nothing to restore. A shorter render is normal
        // (the acronym's tone key is consumed: "JIRA" renders "JỈA").
        if rendered.chars().count() == raw.len()
            && rendered.chars().zip(raw.iter()).all(|(a, &b)| a == b)
        {
            return false;
        }
        if !rendered.chars().all(|c| c.is_alphabetic()) || vowel_runs(rendered) != 1 {
            return false;
        }
        !crate::tables::is_common_vietnamese_syllable(rendered)
    }

    /// The finished token is English even though it needed no restore: it is
    /// already on screen as typed, and it is a context word, a dictionary
    /// word, an ambiguous English word or a doubled-key word.
    fn word_is_plain_english(&self, screen: &str) -> bool {
        let effective = self.effective_word_raw();
        let mut raw = crate::buffers::new_out_buffer();
        for &c in effective.iter() {
            let _ = raw.push(c);
        }
        if screen != &*raw {
            return false;
        }
        crate::tables::is_english_context_word(screen)
            || crate::tables::is_english_override(&effective)
            || crate::tables::is_english_ambiguous_word(&effective)
            || crate::tables::is_english_doubled_key_word(&effective)
    }

    /// How the newest committed word reads on screen — the sentence context
    /// the ambiguous-English rule judges.
    fn prev_word_kind(&self, prev: &CommittedWord) -> PrevWordKind {
        // A Vietnamese word on screen carries marks or đ; an ASCII word is
        // either English or a plain Vietnamese token.
        if !prev
            .rendered
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == '\'')
        {
            return PrevWordKind::Vietnamese;
        }
        // An ALL-CAPS acronym ("CI", "JIRA") is neither English nor Vietnamese
        // context: Vietnamese prose is full of them ("dùng cả CI/CD"). A word
        // committed through an explicit cancel press ("carr" → "car") is the
        // same: plain ASCII, no sentence-language signal.
        if prev.had_cancel || Self::is_all_caps_token(&prev.raw) {
            return PrevWordKind::Neutral;
        }
        // Restored words are on screen as typed. A word whose raw no longer
        // covers its text (it outgrew the keystroke buffer) says nothing
        // about the sentence language.
        if prev.raw.len() < prev.rendered.chars().count() {
            return PrevWordKind::Neutral;
        }
        if prev.raw.len() != prev.rendered.chars().count()
            || !prev
                .raw
                .iter()
                .zip(prev.rendered.chars())
                .all(|(&a, b)| a == b)
        {
            return PrevWordKind::Vietnamese;
        }
        // A top Vietnamese syllable stays Vietnamese evidence even when its
        // letters also spell an English word ("em", "to", "no"): the English
        // tables are built from an inventory, so they read these particles as
        // English. A short one is also a likely English word ("to", "me"), so
        // it is neutral; a longer one keeps the sentence Vietnamese ("anh",
        // "con"). The curated English context words ("the", "is") still win.
        if crate::tables::is_top_vietnamese_syllable(&prev.rendered)
            && !crate::tables::is_english_context_word(&prev.rendered)
        {
            return if prev.rendered.chars().count() <= 2 {
                PrevWordKind::Neutral
            } else {
                PrevWordKind::Vietnamese
            };
        }
        if crate::tables::is_english_context_word(&prev.rendered) {
            return PrevWordKind::English;
        }
        let mut chars = crate::buffers::CharVec::<24>::new();
        for c in prev.rendered.chars() {
            if !chars.try_push(c) {
                return PrevWordKind::Vietnamese;
            }
        }
        // A word this rule (or the dictionary) already restored to raw keeps
        // the chain going: "what is best" — "is" was restored, so "best" is.
        if crate::tables::is_english_override(&chars)
            || crate::tables::is_english_ambiguous_word(&chars)
            || crate::tables::is_english_doubled_key_word(&chars)
        {
            return PrevWordKind::English;
        }
        // A real Vietnamese token keeps the sentence Vietnamese ("than" after
        // "các"), and so does a curated loanword ("file lát", "ok rét").
        // Short function words ("a", "to") are neutral: they are just as
        // likely to be English.
        if prev.rendered.chars().count() > 2
            && crate::tables::is_common_vietnamese_syllable(&prev.rendered)
        {
            return PrevWordKind::Vietnamese;
        }
        if crate::tables::is_vietnamese_loanword(&prev.rendered) {
            return PrevWordKind::Vietnamese;
        }
        PrevWordKind::Neutral
    }

    /// Ambiguous English rule: the finished token is a common English word
    /// whose render is a marked Vietnamese-looking token ("how" → "hơ",
    /// "best" → "bét", "is" → "í"). The Vietnamese rarity score of the render
    /// decides: a reading the word list never contains (pẻ, ák) is English
    /// everywhere, a reading it barely uses (score <= 5: tét, sê, uế) wins
    /// after a neutral word, and every other one needs an English previous
    /// word. With no previous word at all the word keeps its Vietnamese
    /// reading — the retroactive fix restores it once an English word commits.
    fn should_restore_ambiguous_english(&self, ctx: ContextMode) -> bool {
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
        let Some(score) = crate::tables::english_ambiguous_score(&self.diff.word_raw) else {
            return false;
        };
        // The host's recognizer already read the sentence: an English one
        // restores every ambiguous word, whatever its Vietnamese reading
        // scores. A Vietnamese one keeps a common or top Vietnamese syllable.
        if ctx == ContextMode::English {
            return true;
        }
        if Self::keeps_vietnamese_render(ctx, &screen) {
            return false;
        }
        // A render that is a top Vietnamese syllable ("nó", "vẻ", "bú") is a
        // word Vietnamese speakers type all the time: the rarity score is an
        // inventory count, not a frequency, so it cannot judge these. Only a
        // curated English FUNCTION word right before it, separated by exactly
        // one space, restores it ("the car", "my car is red"). Any other
        // English-looking neighbour ("main, có") leaves the Vietnamese word.
        if crate::tables::is_top_vietnamese_syllable(&screen) {
            return self.english_function_context() || self.english_sentence_chain();
        }
        // A render the Vietnamese word list never contains (pẻ, ák) is not a
        // Vietnamese word at all: the English reading wins everywhere.
        if score == 0 {
            return true;
        }
        match self.diff.newest_committed() {
            // Nothing committed yet (a sentence start, or a fresh line): the
            // word stands alone with no context to judge it, so it keeps its
            // Vietnamese reading. A later English word restores it.
            None => false,
            Some(prev) => match self.prev_word_kind(prev) {
                PrevWordKind::English => true,
                PrevWordKind::Neutral => score <= 5,
                PrevWordKind::Vietnamese => false,
            },
        }
    }

    /// The word right before the current one is a curated English FUNCTION
    /// word, separated by exactly one space ("the car", "my car is red").
    ///
    /// A host that does not report separators leaves `gap == 0`: assume the
    /// plain space, the only separator a sentence ever puts there.
    fn english_function_context(&self) -> bool {
        self.diff.newest_committed().is_some_and(|prev| {
            Self::one_space_gap(prev) && crate::tables::is_english_function_word(&prev.rendered)
        })
    }

    /// The two words before the current one both read as English, one of
    /// them a function word, separated by single spaces ("it is the best
    /// car"): the sentence is English, so a
    /// top Vietnamese reading is the wrong one. One English word alone can
    /// sit inside a Vietnamese sentence ("dùng main có"), which is why the
    /// chain needs the second word.
    fn english_sentence_chain(&self) -> bool {
        let Some(prev) = self.diff.committed(0) else {
            return false;
        };
        if !Self::one_space_gap(prev) || self.prev_word_kind(prev) != PrevWordKind::English {
            return false;
        }
        let Some(older) = self.diff.committed(1) else {
            return false;
        };
        if !Self::one_space_gap(older) {
            return false;
        }
        // Two English content words are no proof: Vietnamese developers
        // write "deploy server nó", "test case có", "build fail cả". One of
        // the two must be a curated function word ("the best car").
        if !crate::tables::is_english_function_word(&prev.rendered)
            && !crate::tables::is_english_function_word(&older.rendered)
        {
            return false;
        }
        // A one- or two-letter fragment ("ng", "d") is no sentence-language
        // signal of its own: only a curated function word counts.
        if older.rendered.chars().count() <= 2 {
            return crate::tables::is_english_function_word(&older.rendered);
        }
        self.prev_word_kind(older) == PrevWordKind::English
            || Self::is_plain_english_ascii(&older.rendered)
    }

    /// A word on screen as plain ASCII letters that is no Vietnamese syllable:
    /// in a chain of such words the sentence is English ("freezes its").
    fn is_plain_english_ascii(rendered: &str) -> bool {
        rendered.chars().count() >= 3
            && rendered.chars().all(|c| c.is_ascii_alphabetic())
            && !crate::tables::is_common_vietnamese_syllable(rendered)
            && !crate::tables::is_top_vietnamese_syllable(rendered)
            && !crate::tables::is_vietnamese_loanword(rendered)
    }

    /// A single space separates the word from the next one (`gap == 0` when
    /// the host never reported separators).
    fn one_space_gap(word: &CommittedWord) -> bool {
        word.gap == 0 || (word.gap == 1 && word.sep == ' ')
    }

    /// Retroactive English fix. The word being committed is English, so the
    /// older words in front of it were English too: walk back over at most 3
    /// ring entries that a single separator char follows (`gap == 1`),
    /// skipping neutral ASCII words ("a"), and restore the ambiguous ones
    /// whose render the pipeline marked ("thí" → "this", "í" → "is"). The
    /// walk stops at the first entry that is neither.
    ///
    /// `current_old` is the current word's text on screen and `current_new`
    /// the text it commits as (they differ only when the word is restored).
    /// Returns the backspaces that erase the span from the oldest restored
    /// word to the end of the current word, with the corrected span text
    /// written to `diff_suffix`; the restored entries are updated in place.
    fn retro_restore_english(
        &mut self,
        ctx: ContextMode,
        current_old: &str,
        current_new: &str,
    ) -> Option<usize> {
        // Older entries newest → oldest: rendered, raw, the separator char
        // right after the word, and whether it must be restored.
        let mut older: [Option<(OutBuffer, CharVec<24>, char)>; 3] = [const { None }; 3];
        let mut restored = [false; 3];
        let mut any = false;
        let mut n = 0usize;
        while n < older.len() {
            let Some(entry) = self.diff.committed(n) else {
                break;
            };
            if entry.gap != 1 {
                break;
            }
            // A word typed with an explicit cancel press is on screen as the
            // cancelled render ("carr" → "car"): its raw holds a key the user
            // removed, so it must never be restored back to that raw. A top
            // Vietnamese syllable ("vẻ", "nó", "cả") is never restored either,
            // whatever the score says: it is a word the user very likely
            // meant, so it counts as a non-restorable entry. Across a single
            // '-' the top protection yields: a hyphen compound is one English
            // word ("low-level"), so its Vietnamese-looking half is English.
            // A Vietnamese sentence protects common syllables too, and an
            // English one (the host's recognizer) drops the protection.
            let protected = match ctx {
                ContextMode::English => false,
                ContextMode::Auto => crate::tables::is_top_vietnamese_syllable(&entry.rendered),
                ContextMode::Vietnamese => Self::keeps_vietnamese_render(ctx, &entry.rendered),
            };
            let restorable = !entry.had_cancel
                && (entry.sep == '-' || !protected)
                && (entry.rendered.chars().count() != entry.raw.len()
                    || !entry
                        .raw
                        .iter()
                        .zip(entry.rendered.chars())
                        .all(|(&a, b)| a == b));
            if restorable
                && crate::tables::english_ambiguous_score(&entry.raw)
                    .is_some_and(|score| score <= 25)
            {
                restored[n] = true;
                any = true;
            } else if self.prev_word_kind(entry) != PrevWordKind::Neutral {
                break;
            }
            older[n] = Some((entry.rendered.clone(), entry.raw.clone(), entry.sep));
            n += 1;
        }
        if !any {
            return None;
        }

        // The span as it is on screen and as it must become. A span that does
        // not fit the fixed buffer (three long words) is left alone: a
        // truncated rewrite would corrupt the screen.
        let mut old_span = StackStr::<256>::new();
        let mut new_span = StackStr::<256>::new();
        for step in (0..n).rev() {
            let (rendered, raw, sep) = older[step].as_ref()?;
            let mut ok = old_span.push_str(rendered);
            if restored[step] {
                for &c in raw.iter() {
                    ok &= new_span.push(c);
                }
            } else {
                ok &= new_span.push_str(rendered);
            }
            ok &= old_span.push(*sep);
            ok &= new_span.push(*sep);
            if !ok {
                return None;
            }
        }
        if !old_span.push_str(current_old) || !new_span.push_str(current_new) {
            return None;
        }
        let (bs, _) = Self::diff_into(&old_span, &new_span, &mut self.diff.diff_suffix);

        for (step, entry) in older.iter().enumerate().take(n) {
            if !restored[step] {
                continue;
            }
            let Some((_, raw, _)) = entry else { continue };
            let mut rendered = crate::buffers::new_out_buffer();
            for &c in raw.iter() {
                let _ = rendered.push(c);
            }
            if let Some(entry) = self.diff.committed_mut(step) {
                entry.rendered = rendered;
            }
        }
        Some(bs)
    }
}
