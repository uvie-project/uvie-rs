//! Committed-word ring geometry, separator accounting and `backspace_at`.
//!
//! The host measures its caret offset (`caret_back`, screen chars) from the
//! END of the NEWEST committed word. Every ring entry carries the separator
//! chars that follow it on screen, so the word-end offsets are
//! `end(0) = 0`, `end(k+1) = end(k) + rendered_len(k) + gap(k+1)`. These tests
//! pin that geometry, the `note_separator`/`remove_separator` accounting and
//! the in-place `edit_at`/`backspace_at` updates (no popping, no composing).

use uvie::UltraFastViEngine;
use uvie::diff::Diffable;

/// Type `word`, commit it and post one separator (the host posts the char
/// itself; the engine only records it).
fn commit_word(e: &mut UltraFastViEngine, word: &str, sep: char) {
    for c in word.chars() {
        let _ = e.feed_diff(c);
    }
    let _ = e.feed_diff(sep);
}

#[test]
fn separator_accounting() {
    let mut e = UltraFastViEngine::new();
    // No committed word: separators are not tracked.
    assert_eq!(e.trailing_gap(), 0);
    e.note_separator();
    assert_eq!(e.trailing_gap(), 0);
    assert!(!e.remove_separator());

    commit_word(&mut e, "ab", ' ');
    assert_eq!(e.trailing_gap(), 1);
    e.note_separator();
    e.note_separator();
    assert_eq!(e.trailing_gap(), 3);
    assert!(e.remove_separator());
    assert!(e.remove_separator());
    assert!(e.remove_separator());
    assert_eq!(e.trailing_gap(), 0);
    assert!(!e.remove_separator(), "nothing left to account for");
}

#[test]
fn idle_commit_records_nothing() {
    // A boundary key pressed with nothing composing must not push a phantom
    // entry: the caret geometry is measured from the real word.
    let mut e = UltraFastViEngine::new();
    commit_word(&mut e, "ab", ' ');
    let (bs, out) = e.commit_diff();
    assert_eq!((bs, out), (0, ""));
    // The newest entry is still "ab" (caret_back 0 edits it, not a phantom).
    let (bs, fwd, out) = e.edit_at_caret_diff(0, 's').expect("edit the word");
    assert_eq!((bs, fwd), (0, 0));
    assert_eq!(out, "s");
}

#[test]
fn word_end_geometry_uses_reported_gaps() {
    // "ab cd eg " — one separator after each word.
    let mut e = UltraFastViEngine::new();
    for word in ["ab", "cd", "eg"] {
        commit_word(&mut e, word, ' ');
    }
    assert_eq!(e.trailing_gap(), 1);

    // end(1) = len("eg") + gap("cd") = 2 + 1 = 3.
    let (bs, fwd, out) = e.edit_at_caret_diff(3, 's').expect("cd's end");
    assert_eq!((bs, fwd), (0, 0));
    assert_eq!(out, "s", "cd -> cds needs no backspace");
    assert_eq!(e.last_caret_back_after_edit(), 3);

    // end(2) = end(1) + len("cds") + gap("ab") = 3 + 3 + 1 = 7.
    let (bs, fwd, out) = e.edit_at_caret_diff(7, 'x').expect("ab's end");
    assert_eq!((bs, fwd), (0, 0));
    assert_eq!(out, "x");
    assert_eq!(e.last_caret_back_after_edit(), 7);

    // Neither edit touched the other entries: "cds" and "eg" are still there.
    let (bs, fwd, out) = e.edit_at_caret_diff(3, 's').expect("cds' end");
    assert_eq!((bs, fwd), (0, 0));
    assert_eq!(out, "s", "cds -> cdss");
}

#[test]
fn double_separator_geometry() {
    // "ab  cd " — two spaces after "ab", one after "cd".
    let mut e = UltraFastViEngine::new();
    commit_word(&mut e, "ab", ' ');
    // A second Space with nothing composing only grows the gap.
    let (bs, out) = e.feed_diff(' ');
    assert_eq!((bs, out), (0, " "));
    commit_word(&mut e, "cd", ' ');
    assert_eq!(e.trailing_gap(), 1);

    // end(1) = len("cd") + gap("ab") = 2 + 2 = 4.
    let (bs, fwd, out) = e.edit_at_caret_diff(4, 's').expect("ab's end");
    assert_eq!((bs, fwd), (0, 0));
    assert_eq!(out, "s");
    // The gap itself is not an editable position.
    assert!(e.edit_at_caret_diff(3, 's').is_none());
}

#[test]
fn off_boundary_carets_decline() {
    let mut e = UltraFastViEngine::new();
    for word in ["ab", "ban", "cd"] {
        commit_word(&mut e, word, ' ');
    }
    // end(1) = len("cd") + gap("ban") = 2 + 1 = 3; 2 is the separator after
    // the newest word.
    assert!(e.edit_at_caret_diff(2, 's').is_none());
    // end(2) = end(1) + len("ban") + gap("ab") = 3 + 3 + 1 = 7; past the
    // oldest word end nothing matches.
    assert!(e.edit_at_caret_diff(9, 's').is_none());
    // Mid-word still edits (caret_back 4 → "ba|n"): a tone key rewrites the
    // whole word, erasing the prefix left of the caret and the tail.
    let (bs, fwd, out) = e.edit_at_caret_diff(4, 's').expect("mid-word edit");
    assert_eq!((bs, fwd), (2, 1));
    assert_eq!(out, "bán");
}

#[test]
fn boundary_chars_keep_the_ring() {
    // Punctuation commits go through the same path and must not wipe the
    // history: "chao, ban " leaves "chao" editable.
    let mut e = UltraFastViEngine::new();
    for c in "chao".chars() {
        let _ = e.feed_diff(c);
    }
    let (_, out) = e.feed_diff(',');
    assert_eq!(out, ",");
    for c in "ban".chars() {
        let _ = e.feed_diff(c);
    }
    let _ = e.feed_diff(' ');

    // end(1) = len("ban") + gap("chao") = 3 + 1 = 4.
    let (bs, fwd, out) = e.edit_at_caret_diff(4, 'f').expect("chao's end");
    assert_eq!((bs, fwd), (2, 0));
    assert_eq!(out, "ào", "chao -> chào");
}

#[test]
fn backspace_at_undoes_last_keystroke() {
    let mut e = UltraFastViEngine::new();
    commit_word(&mut e, "viet", ' ');
    let (bs, out) = e.backspace_at_caret_diff(0).expect("word end");
    assert_eq!((bs, out), (1, ""), "viet -> vie");
    assert_eq!(e.last_caret_back_after_edit(), 0);
    // The engine stays idle and the entry is updated in place: retyping the
    // popped key restores the word.
    assert!(!e.is_composing_diff());
    let (bs, fwd, out) = e.edit_at_caret_diff(0, 't').expect("edited word");
    assert_eq!((bs, fwd), (0, 0));
    assert_eq!(out, "t", "vie + t -> viet again");

    // Mid-word carets decline: there is no keystroke to undo there.
    assert!(e.backspace_at_caret_diff(1).is_none());
    // So does an empty ring.
    let mut empty = UltraFastViEngine::new();
    assert!(empty.backspace_at_caret_diff(0).is_none());
}

#[test]
fn backspace_at_emptying_the_newest_word_merges_its_gap() {
    // "a b " — deleting the newest word leaves the caret inside the merged
    // separators, one right of the new newest word's end.
    let mut e = UltraFastViEngine::new();
    commit_word(&mut e, "a", ' ');
    commit_word(&mut e, "b", ' ');
    let (bs, out) = e.backspace_at_caret_diff(0).expect("word end");
    assert_eq!((bs, out), (1, ""), "the whole word is erased");
    assert_eq!(
        e.last_caret_back_after_edit(),
        -1,
        "the caret sits one right of the new newest word's end"
    );
    assert_eq!(e.trailing_gap(), 2, "both separators now follow \"a\"");
    // "a" is the newest entry now, at caret_back 0.
    let (bs, fwd, out) = e.edit_at_caret_diff(0, 's').expect("a's end");
    assert_eq!((bs, fwd), (1, 0));
    assert_eq!(out, "á");
}

#[test]
fn backspace_at_emptying_an_older_word_keeps_the_offset() {
    // "a b c " — erase the middle word; the caret keeps its absolute offset.
    let mut e = UltraFastViEngine::new();
    for word in ["a", "b", "c"] {
        commit_word(&mut e, word, ' ');
    }
    // end(1) = len("c") + gap("b") = 1 + 1 = 2.
    let (bs, out) = e.backspace_at_caret_diff(2).expect("b's end");
    assert_eq!((bs, out), (1, ""));
    assert_eq!(e.last_caret_back_after_edit(), 2);
    assert_eq!(e.trailing_gap(), 1);
    // The surviving older word absorbed the removed gap: end(1) is now
    // len("c") + gap("a") = 1 + 2 = 3.
    let (bs, fwd, out) = e.edit_at_caret_diff(3, 's').expect("a's end");
    assert_eq!((bs, fwd), (1, 0));
    assert_eq!(out, "á");
}

#[test]
fn reset_clears_the_ring_and_the_gap() {
    let mut e = UltraFastViEngine::new();
    commit_word(&mut e, "ab", ' ');
    e.reset_diff();
    assert_eq!(e.trailing_gap(), 0);
    assert!(e.edit_at_caret_diff(0, 's').is_none());
    assert!(e.backspace_at_caret_diff(0).is_none());
}
