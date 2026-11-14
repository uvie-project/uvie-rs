//! Post-commit word editing (`edit_at_caret_diff`) — LabanKey-style:
//! type words, commit them, arrow the caret back onto any committed word's
//! end, then type a tone/modifier key to re-render that word in place.
//!
//! Core contract: editing a committed word with key `ch` must produce exactly
//! the same on-screen text as typing the extended raw word fresh, with every
//! other word on screen untouched.

use uvie::diff::Diffable;
use uvie::{InputMethod, UltraFastViEngine};

/// Screen model driven through the real engine: full on-screen text plus a
/// caret offset. The engine's (backspaces, suffix) diffs apply at the caret;
/// the commit space is posted natively by the host (after the caret).
struct Screen {
    text: String,
    caret: usize,
    /// Absolute offset of the anchor: the end of the engine's newest text
    /// (the newest committed word when idle, the composing word otherwise).
    anchor: usize,
    engine: UltraFastViEngine,
}

impl Screen {
    fn new(modern: bool) -> Self {
        let mut engine = UltraFastViEngine::new();
        engine.set_input_method(InputMethod::Telex);
        engine.set_modern_orthography(modern);
        Self {
            text: String::new(),
            caret: 0,
            anchor: 0,
            engine,
        }
    }

    /// Screen with the English override (and with it the word-boundary
    /// restore-raw rule) off: V-C-V tokens like "neebo" stay rendered
    /// ("nêbo") instead of being restored to raw at commit.
    fn new_no_restore(modern: bool) -> Self {
        let mut s = Self::new(modern);
        s.engine.set_english_override(false);
        s
    }

    /// Apply a (backspaces, suffix) instruction at the caret.
    fn apply(&mut self, bs: usize, out: &str) {
        let kept = self.caret.saturating_sub(bs);
        let head: String = self.text.chars().take(kept).collect();
        let tail: String = self.text.chars().skip(self.caret).collect();
        self.text = format!("{head}{out}{tail}");
        self.caret = kept + out.chars().count();
        if self.anchor < self.caret {
            self.anchor = self.caret;
        }
    }

    fn feed(&mut self, ch: char) {
        let (bs, out) = self.engine.feed_diff(ch);
        let out = out.to_string();
        self.apply(bs, &out);
    }

    /// Type `word` and commit it. The host posts the commit space natively,
    /// so the caret ends up 1 right of the committed word's end.
    fn commit_word(&mut self, word: &str) {
        for c in word.chars() {
            self.feed(c);
        }
        self.commit_composing();
    }

    /// Commit the engine's current composing word (e.g. an edited word) and
    /// post the commit space natively — the space is inserted at the caret.
    fn commit_composing(&mut self) {
        // Apply the commit plan like the real host: the restore-raw rule
        // can rewrite the word at the boundary.
        let (bs, out) = self.engine.commit_diff();
        let out = out.to_string();
        self.apply(bs, &out);
        let head: String = self.text.chars().take(self.caret).collect();
        let tail: String = self.text.chars().skip(self.caret).collect();
        self.text = format!("{head} {tail}");
        self.caret += 1;
        self.anchor = self.caret - 1;
        // The host posts the space natively and reports it as a separator, so
        // the engine's caret geometry knows about it.
        self.engine.note_separator();
    }

    /// Arrow-left: the caret steps back 1 char (the engine is not involved).
    fn arrow_left(&mut self) {
        self.caret = self.caret.saturating_sub(1);
    }

    /// Arrow-right: the caret steps forward 1 char.
    fn arrow_right(&mut self) {
        if self.caret < self.text.chars().count() {
            self.caret += 1;
        }
    }

    /// The caret's distance back to the anchor (what the host passes to
    /// `edit_at_caret_diff`); 0 = the caret at the newest word's end.
    fn caret_back(&self) -> usize {
        self.anchor.saturating_sub(self.caret)
    }

    /// Apply a (backspaces, forward_deletes, suffix) edit at the caret:
    /// erase `bs` chars left of the caret and `fwd_del` chars right of it,
    /// then insert `out`.
    fn apply_edit(&mut self, bs: usize, fwd_del: usize, out: &str) {
        let kept = self.caret.saturating_sub(bs);
        let head: String = self.text.chars().take(kept).collect();
        let tail: String = self.text.chars().skip(self.caret + fwd_del).collect();
        self.text = format!("{head}{out}{tail}");
        self.caret = kept + out.chars().count();
        if self.anchor < self.caret {
            self.anchor = self.caret;
        }
    }

    /// Re-enter a committed word with `ch` appended to its raw keystrokes.
    /// `caret_back` must land exactly on the target word's end boundary.
    fn edit_at(&mut self, caret_back: usize, ch: char) -> Option<String> {
        let (bs, fwd_del, out) = self.engine.edit_at_caret_diff(caret_back, ch)?;
        let out = out.to_string();
        self.apply_edit(bs, fwd_del, &out);
        // The caret lands at the edited word's end (the engine stays idle and
        // reports the same offset); re-root the anchor there.
        self.anchor = self.caret;
        Some(self.text.clone())
    }

    /// Edit the newest committed word (caret_back 0).
    fn edit(&mut self, ch: char) -> Option<String> {
        self.edit_at(0, ch)
    }

    /// Full engine reset (also clears the committed-word history).
    fn reset(&mut self) {
        self.engine.reset_diff();
        self.text.clear();
        self.caret = 0;
        self.anchor = 0;
    }
}

/// Fresh-typing oracle: type `word` + edit key `ch` + commit space on a
/// clean engine, and return the full on-screen text.
fn type_fresh(word: &str, ch: char) -> String {
    let mut s = Screen::new(true);
    for ch in format!("{word}{ch}").chars() {
        s.feed(ch);
    }
    let _ = s.engine.commit_diff();
    s.text.push(' ');
    s.text
}

#[test]
fn edit_tone_key_matches_fresh_typing() {
    let cases: &[(&str, char)] = &[
        ("don", 's'),
        ("don", 'f'),
        ("don", 'j'),
        ("don", 'r'),
        ("don", 'x'),
        ("vieetj", 's'),
        ("vieetj", 'f'),
        ("ddoong", 's'),
        ("thuocs", 'f'),
        ("nguonf", 's'),
        ("choas", 'f'),
        ("phoos", 'x'),
        ("dduwowcj", 'f'),
        ("neebo", 's'),
    ];
    for &(word, ch) in cases {
        let fresh = type_fresh(word, ch);
        // V-C-V tokens are restored to raw at commit by default.
        let mut s = Screen::new_no_restore(true);
        s.commit_word(word);
        s.arrow_left(); // caret onto the committed word's end
        let Some(edited) = s.edit(ch) else {
            panic!("edit of {word:?} with {ch:?} was not handled");
        };
        assert_eq!(
            edited, fresh,
            "editing {word:?} with {ch:?} must equal fresh typing"
        );
    }
}

#[test]
fn edit_english_override_word() {
    // "good" is in the English override dictionary: the commit shows "good".
    let mut s = Screen::new(true);
    s.commit_word("good");
    assert!(
        s.text.starts_with("good"),
        "override must show the raw English word"
    );

    s.arrow_left();
    let edited = s.edit('s').expect("edit handled");
    // Fresh typing of "goods": the override fires at "good", then 's' extends.
    let fresh = type_fresh("good", 's');
    assert_eq!(edited, fresh, "edit must match fresh typing");
    assert!(edited.starts_with("goods"));
}

#[test]
fn edit_targets_newest_committed_word() {
    let mut s = Screen::new(true);
    s.commit_word("vie");
    s.commit_word("con");
    s.arrow_left();
    let edited = s.edit('s').expect("edit handled");
    // Editing targets "con" (the newest), not the first word; the earlier
    // text and the commit space are untouched.
    let fresh = type_fresh("con", 's');
    assert!(
        edited.ends_with(&fresh),
        "{edited:?} must end with {fresh:?}"
    );
    assert!(edited.starts_with("vie"), "earlier word must be untouched");
}

#[test]
fn edit_second_word_back_in_three_word_sentence() {
    // "ab cd ef " — edit "cd" (the middle word): the caret sits at cd's end,
    // which is len("eg") + 1 boundary char behind the anchor.
    let mut s = Screen::new(true);
    s.commit_word("ab");
    s.commit_word("cd");
    s.commit_word("eg");
    let back = "eg".chars().count() + 1;
    for _ in 0..back + 1 {
        s.arrow_left(); // -1 (post-commit) + steps onto cd's end
    }
    assert_eq!(s.caret_back(), back, "caret must sit on cd's end boundary");

    let edited = s.edit_at(s.caret_back(), 's').expect("edit handled");
    // "cd" + 's' → "cds"; the older "ab" and the newer "eg" untouched.
    assert!(edited.contains("ab cds eg "), "got {edited:?}");
}

#[test]
fn edit_oldest_word_back_in_three_word_sentence() {
    // "ab cd ef " — edit "ab" (the oldest ring word): its end boundary is
    // len(ef) + 1 + len(cd) + 1 chars behind the anchor.
    let mut s = Screen::new(true);
    s.commit_word("ab");
    s.commit_word("cd");
    s.commit_word("eg");
    let back = "eg".chars().count() + 1 + "cd".chars().count() + 1;
    for _ in 0..back + 1 {
        s.arrow_left();
    }
    assert_eq!(s.caret_back(), back, "caret must sit on ab's end boundary");
    let edited = s.edit_at(s.caret_back(), 's').expect("edit handled");
    // "ab" + 's' → "abs"; the newer words untouched.
    assert!(edited.starts_with("abs cd eg "), "got {edited:?}");
}

#[test]
fn edit_off_boundary_caret_declines() {
    // A caret position between boundaries (mid-word) must not fire an edit.
    let mut s = Screen::new(true);
    s.commit_word("ab");
    s.commit_word("cd");
    // Boundary for "ab"'s end = len("cd") + 1 = 3; caret_back 2 (between the
    // space and "cd") is off-boundary.
    for _ in 0..("cd".chars().count() + 1) {
        s.arrow_left();
    }
    assert_eq!(s.caret_back(), 2);
    assert!(
        s.edit_at(s.caret_back(), 'x').is_none(),
        "off-boundary caret must not edit"
    );
}

#[test]
fn edit_older_word_keeps_newer_entries() {
    // "ab cd eg " — edit "cd"; the newer "eg" and the older "ab" both stay in
    // the ring and remain editable at their own boundaries.
    let mut s = Screen::new(true);
    s.commit_word("ab");
    s.commit_word("cd");
    s.commit_word("eg");
    for _ in 0..("eg".chars().count() + 1 + 1) {
        s.arrow_left(); // onto cd's end
    }
    let edited = s.edit_at(s.caret_back(), 's').expect("edit handled");
    assert!(edited.contains("cds"), "cd edited, got {edited:?}");

    // The newer word is still the newest entry: 3 rendered chars right of
    // "cds"'s end is its own end (caret_back 0).
    for _ in 0.."cds".chars().count() {
        s.arrow_right();
    }
    assert_eq!(s.caret_back(), 0);
    let edited = s.edit('s').expect("newer word still editable");
    assert!(edited.contains("egs"), "got {edited:?}");

    // The older word is still editable too: its end is "egs" (3) + its gap
    // (1) + "cds" (3) + its gap (1) behind the anchor.
    let back = "egs".chars().count() + 1 + "cds".chars().count() + 1;
    for _ in 0..back {
        s.arrow_left();
    }
    assert_eq!(s.caret_back(), back);
    let edited = s
        .edit_at(s.caret_back(), 'x')
        .expect("older word still editable");
    assert!(edited.contains("abx"), "got {edited:?}");
}

#[test]
fn edit_without_committed_word_returns_none() {
    let mut s = Screen::new(true);
    assert!(s.edit('s').is_none(), "empty history must not edit");
    for ch in "don".chars() {
        s.feed(ch);
    }
    // Still composing — no committed word yet.
    assert!(s.edit('s').is_none(), "composing word must not edit");
}

#[test]
fn reset_clears_edit_history() {
    let mut s = Screen::new(true);
    s.commit_word("don");
    s.reset();
    assert!(s.edit('s').is_none(), "reset must clear edit history");
}

#[test]
fn backspace_after_edit_undoes_the_edit() {
    let mut s = Screen::new(true);
    s.commit_word("don");
    s.arrow_left();
    let _ = s.edit('s').expect("edit handled");
    assert_eq!(s.text, "dón ");
    // The engine stays idle after an edit (the entry was updated in place),
    // so a backspace at the caret — the edited word's end — undoes the last
    // keystroke and restores the pre-edit word.
    assert!(!s.engine.is_composing_diff());
    assert_eq!(s.caret_back(), 0);
    let (bs, out) = s
        .engine
        .backspace_at_caret_diff(s.caret_back())
        .expect("backspace at the word end must re-open the word");
    let out = out.to_string();
    s.apply(bs, &out);
    assert_eq!(
        s.text, "don ",
        "backspace after edit must restore the pre-edit word"
    );
}

#[test]
fn ring_eviction_keeps_newest_editable() {
    let mut s = Screen::new(true);
    // Commit more words than the ring capacity (8).
    for i in 0..12 {
        s.commit_word(&format!("wo{i}"));
    }
    s.arrow_left();
    let edited = s.edit('s').expect("edit handled");
    // The newest committed word is "wo11"; editing it must match fresh typing.
    let fresh = type_fresh("wo11", 's');
    assert!(
        edited.ends_with(&fresh),
        "{edited:?} must end with {fresh:?}"
    );
}

// ------------------------------------------------------------------
// Mid-word caret editing
// ------------------------------------------------------------------

#[test]
fn mid_word_tone_edit_newest_word() {
    let mut s = Screen::new(true);
    s.commit_word("viet");
    // Arrow left twice: caret lands inside "viet" after "vie"
    // (caret_back 1 → 3 rendered chars before the caret).
    s.arrow_left();
    s.arrow_left();
    let edited = s
        .edit_at(s.caret_back(), 's')
        .expect("mid-word edit handled");
    assert_eq!(edited, "viết ");
}

#[test]
fn mid_word_tone_edit_older_word() {
    let mut s = Screen::new(true);
    s.commit_word("toa");
    s.commit_word("ngan");
    // 7 arrows: "toa " is 4 chars + "ngan" 4 → caret lands inside "toa"
    // after "to" (caret_back 6; "toa" end boundary is 5).
    for _ in 0..7 {
        s.arrow_left();
    }
    let cb = s.caret_back();
    let edited = s.edit_at(cb, 'f').expect("mid-word edit on older word");
    assert_eq!(edited, "toà ngan ");
}

#[test]
fn mid_word_edit_word_start_declines() {
    // Caret before the word's first char is a space/word junction, not a
    // mid-word position: no edit may fire.
    let mut s = Screen::new(true);
    s.commit_word("viet");
    for _ in 0..5 {
        s.arrow_left();
    }
    assert_eq!(s.caret_back(), 4);
    assert!(s.edit_at(s.caret_back(), 's').is_none());
}

#[test]
fn mid_word_literal_insert() {
    // A non-tone key inserts at the caret's keystroke position.
    let mut s = Screen::new(true);
    s.commit_word("vit");
    // Caret after "vi" (caret_back 1 inside "vit"): insert 'e'.
    for _ in 0..2 {
        s.arrow_left();
    }
    let edited = s
        .edit_at(s.caret_back(), 'e')
        .expect("mid-word edit handled");
    assert_eq!(edited, "viet ");
}

#[test]
fn mid_word_tone_older_syllable_of_vcv_word() {
    // V-C-V word "neebo" → "nêbo" (splits into "nê"+"bo").
    let mut s = Screen::new_no_restore(true);
    s.commit_word("neebo");
    // Caret after "nê" (caret_back 2 → 2 rendered chars before the caret):
    // 'f' snaps to the END of the first syllable's raw run — inserting at
    // the caret split would produce "neef" mid-coda instead of "nè".
    for _ in 0..3 {
        s.arrow_left();
    }
    let edited = s
        .edit_at(s.caret_back(), 'f')
        .expect("mid-word tone on first syllable");
    assert_eq!(edited, "nềbo ");
}

#[test]
fn mid_word_tone_last_syllable_of_vcv_word() {
    // Same word; caret inside "bo" (after "nêb") → tone the LAST syllable.
    let mut s = Screen::new_no_restore(true);
    s.commit_word("neebo");
    for _ in 0..2 {
        s.arrow_left();
    }
    let edited = s
        .edit_at(s.caret_back(), 'f')
        .expect("mid-word tone on last syllable");
    assert_eq!(edited, "nêbò ");
}

#[test]
fn mid_word_edit_composing_declined() {
    // Engine composing (not idle) → no edit, matching the existing gate.
    let mut s = Screen::new(true);
    s.commit_word("viet");
    for _ in 0..3 {
        s.arrow_left();
    }
    s.feed('x');
    assert!(s.edit_at(1, 's').is_none());
}
