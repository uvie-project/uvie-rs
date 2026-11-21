//! Cross-word rewrite safety: the retroactive English restore and the
//! contraction fix erase and retype text left of the caret, so they must
//! only run when the committed-word ring describes that text exactly.
//!
//! A. `remove_separator` marks a mixed gap's surviving separator unknown
//!    ('\0') and re-establishes the separator history when the gap empties.
//! B. No rewrite crosses a control or unknown separator (newline, the Break
//!    key's '\0', a mixed-gap leftover).
//! C. An overflowed word (`editable = false`) declines the contraction and
//!    the retro restore: its raw is a tail, not the word.
//! D. `set_cross_word_rewrite(false)` (preedit hosts) disables both
//!    cross-word rewrites while single-word restores keep working.

use uvie::UltraFastViEngine;
use uvie::diff::Diffable;

/// Screen buffer + the host half of the wire protocol: typing feeds the
/// engine, a committed boundary calls `commit_with_boundary_diff` and the
/// host reports the separator it inserted.
struct Host {
    screen: String,
}

impl Host {
    fn new() -> Self {
        Self {
            screen: String::new(),
        }
    }

    fn apply(&mut self, bs: usize, suffix: &str) {
        for _ in 0..bs {
            self.screen.pop();
        }
        self.screen.push_str(suffix);
    }

    /// The app inserts `sep` itself after the engine's commit plan ran.
    fn commit(&mut self, e: &mut UltraFastViEngine, sep: char) -> (usize, String) {
        let (bs, suffix) = e.commit_with_boundary_diff(sep);
        let suffix = suffix.to_owned();
        self.apply(bs, &suffix);
        e.note_separator(sep);
        self.screen.push(sep);
        (bs, suffix)
    }

    fn type_text(&mut self, e: &mut UltraFastViEngine, text: &str) {
        for c in text.chars() {
            let (bs, suffix) = e.feed_diff(c);
            self.apply(bs, suffix);
        }
    }

    /// The app deletes one separator char natively; the engine only tracks it.
    fn backspace_separator(&mut self, e: &mut UltraFastViEngine) {
        assert!(e.remove_separator());
        self.screen.pop();
    }
}

/// A. Deleting one char from a mixed separator gap leaves the surviving char
/// unknown: no rewrite may replace it ("this, " losing the space leaves
/// "this," - the comma must survive).
#[test]
fn remove_separator_on_mixed_gap_marks_sep_unknown() {
    let mut e = UltraFastViEngine::new();
    let mut h = Host::new();
    h.type_text(&mut e, "this");
    h.commit(&mut e, ',');
    h.commit(&mut e, ' ');
    h.backspace_separator(&mut e);
    h.type_text(&mut e, "is a test");
    h.commit(&mut e, ' ');
    assert_eq!(h.screen, "thí,is a test ");
}

/// A. Two identical separators shrink to one exact char: the rewrite still
/// sees a space and the English context keeps working.
#[test]
fn remove_separator_on_uniform_gap_keeps_exact_sep() {
    let mut e = UltraFastViEngine::new();
    let mut h = Host::new();
    h.type_text(&mut e, "this");
    h.commit(&mut e, ' ');
    h.commit(&mut e, ' ');
    h.backspace_separator(&mut e);
    h.type_text(&mut e, "is a test");
    h.commit(&mut e, ' ');
    assert_eq!(h.screen, "this is a test ");
}

/// B. A newline separator can never be rebuilt by a keystroke rewrite (in a
/// chat app Enter already sent the line), so the walk stops before it.
#[test]
fn newline_separator_blocks_cross_word_rewrite() {
    let mut e = UltraFastViEngine::new();
    let mut h = Host::new();
    h.type_text(&mut e, "this");
    h.commit(&mut e, '\n');
    h.type_text(&mut e, "test");
    let (bs, suffix) = h.commit(&mut e, ' ');
    assert!(bs <= 4, "bs={bs} would reach past the newline");
    assert!(!suffix.contains('\n'));
    assert_eq!(h.screen.lines().next_back().unwrap(), "test ");
}

/// B. The Break key reports '\0': unknowable, like a mixed-gap leftover.
#[test]
fn break_separator_blocks_cross_word_rewrite() {
    let mut e = UltraFastViEngine::new();
    let mut h = Host::new();
    h.type_text(&mut e, "this");
    h.commit(&mut e, '\0');
    h.type_text(&mut e, "test");
    let (bs, suffix) = h.commit(&mut e, ' ');
    assert!(bs <= 4, "bs={bs} would reach past the unknown separator");
    assert!(!suffix.contains("this"));
    assert!(h.screen.ends_with("test "), "{:?}", h.screen);
}

/// C. A word longer than the 24-key raw buffer only knows its tail: the
/// contraction rewrite must not rebuild it, and the word stays intact.
#[test]
fn overflowed_word_blocks_contraction_and_retro() {
    let mut e = UltraFastViEngine::new();
    let mut h = Host::new();
    h.type_text(&mut e, "getCurrentApplicationSettings");
    h.commit(&mut e, '\'');
    h.type_text(&mut e, "s");
    h.commit(&mut e, ' ');
    assert_eq!(h.screen, "getCurrentApplicationSettings's ");
}

/// C. The same overflowed word without the apostrophe is untouched.
#[test]
fn overflowed_word_alone_stays_intact() {
    let mut e = UltraFastViEngine::new();
    let mut h = Host::new();
    h.type_text(&mut e, "getCurrentApplicationSettings");
    h.commit(&mut e, ' ');
    assert_eq!(h.screen, "getCurrentApplicationSettings ");
}

/// D. A preedit host cannot rewrite committed text: the ambiguous words
/// stay Vietnamese while the strong dictionary word still restores alone.
#[test]
fn cross_word_rewrite_off_keeps_single_word_restore() {
    let mut e = UltraFastViEngine::new();
    e.set_cross_word_rewrite(false);
    let mut h = Host::new();
    for word in ["this", "is", "a", "test"] {
        h.type_text(&mut e, word);
        let (bs, _) = h.commit(&mut e, ' ');
        // No plan may erase more than the word just committed.
        assert!(bs <= word.len(), "{word}: bs={bs}");
    }
    assert_eq!(h.screen, "thí í a test ");
}

/// D. With the flag on the same sentence retro-restores every ambiguous word.
#[test]
fn cross_word_rewrite_on_restores_the_sentence() {
    let mut e = UltraFastViEngine::new();
    let mut h = Host::new();
    for word in ["this", "is", "a", "test"] {
        h.type_text(&mut e, word);
        h.commit(&mut e, ' ');
    }
    assert_eq!(h.screen, "this is a test ");
}

/// D. The flag also gates the contraction rewrite: the suffix commits on the
/// normal path instead of rewriting the previous word.
#[test]
fn cross_word_rewrite_off_blocks_contraction() {
    let mut e = UltraFastViEngine::new();
    e.set_cross_word_rewrite(false);
    let mut h = Host::new();
    h.type_text(&mut e, "vieetj");
    h.commit(&mut e, '\'');
    h.type_text(&mut e, "s");
    let (bs, suffix) = h.commit(&mut e, ' ');
    assert!(bs <= 1, "bs={bs} would rewrite the previous word");
    assert!(!suffix.contains("vieetj"));
}

/// A. `sentence_text` never embeds a NUL: the unknown separator left by a
/// mixed-gap delete stands in as a space, like the unrecorded ones.
#[test]
fn sentence_text_stands_in_space_for_unknown_separator() {
    let mut e = UltraFastViEngine::new();
    let mut h = Host::new();
    h.type_text(&mut e, "xin");
    h.commit(&mut e, ',');
    h.commit(&mut e, ' ');
    h.backspace_separator(&mut e);
    h.type_text(&mut e, "chaof");
    h.commit(&mut e, ' ');
    let text = e.sentence_text();
    assert!(!text.contains('\0'), "{text:?}");
    assert!(text.contains("xin"), "{text:?}");
    assert!(text.contains("chào"), "{text:?}");
}

/// A. A deleted '.' never confirmed a sentence boundary: once the separator
/// is gone the next word must not find the sentence ended.
#[test]
fn remove_separator_drops_the_pending_sentence_break() {
    let mut e = UltraFastViEngine::new();
    let mut h = Host::new();
    h.type_text(&mut e, "chaof");
    h.commit(&mut e, '.');
    h.backspace_separator(&mut e);
    h.type_text(&mut e, "dduwowcj");
    h.commit(&mut e, ' ');
    let text = e.sentence_text();
    assert!(text.contains("chào"), "{text:?}");

    // Control: without the delete the '.' + ' ' ends the sentence at
    // "chào", and the walk stops before it.
    let mut e = UltraFastViEngine::new();
    let mut h = Host::new();
    h.type_text(&mut e, "chaof");
    h.commit(&mut e, '.');
    h.commit(&mut e, ' ');
    h.type_text(&mut e, "dduwowcj");
    h.commit(&mut e, ' ');
    let text = e.sentence_text();
    assert!(text.contains("được"), "{text:?}");
    assert!(!text.contains("chào"), "{text:?}");
}

/// Exact undo: `remove_last_separator` restores the separator state the '.'
/// created - the code-token dot is gone, so the next word renders.
#[test]
fn remove_last_separator_undoes_code_token_dot() {
    // Control: a word glued after the code dot shows its raw keystrokes.
    let mut e = UltraFastViEngine::new();
    let mut h = Host::new();
    h.type_text(&mut e, "chaof");
    h.commit(&mut e, '.');
    h.type_text(&mut e, "dduwowcj");
    h.commit(&mut e, ' ');
    assert_eq!(h.screen, "chào.dduwowcj ");

    // Deleting the dot undoes its note_separator exactly: the next word is
    // no longer inside a code token and renders as Vietnamese.
    let mut e = UltraFastViEngine::new();
    let mut h = Host::new();
    h.type_text(&mut e, "chaof");
    h.commit(&mut e, '.');
    assert!(e.remove_last_separator());
    h.screen.pop();
    h.type_text(&mut e, "dduwowcj");
    h.commit(&mut e, ' ');
    assert_eq!(h.screen, "chàođược ");
}

/// Exact undo on a mixed gap keeps the surviving separator known exactly:
/// the retro fix sees the ',' and restores the word across it.
#[test]
fn remove_last_separator_keeps_mixed_gap_sep_known() {
    let mut e = UltraFastViEngine::new();
    let mut h = Host::new();
    h.type_text(&mut e, "this");
    h.commit(&mut e, ',');
    h.commit(&mut e, ' ');
    assert!(e.remove_last_separator());
    h.screen.pop();
    h.type_text(&mut e, "is a test");
    h.commit(&mut e, ' ');
    assert_eq!(h.screen, "this,is a test ");
}

/// The undo stack holds four separators: past that the removal falls back
/// to the conservative path - still returns true, the gap still shrinks, and
/// the next word still renders safely.
#[test]
fn remove_last_separator_falls_back_past_stack_depth() {
    let mut e = UltraFastViEngine::new();
    let mut h = Host::new();
    h.type_text(&mut e, "chaof");
    for _ in 0..6 {
        h.commit(&mut e, ' ');
    }
    for _ in 0..6 {
        assert!(e.remove_last_separator());
        h.screen.pop();
    }
    assert!(!e.remove_last_separator());
    h.type_text(&mut e, "dduwowcj");
    h.commit(&mut e, ' ');
    assert_eq!(h.screen, "chàođược ");
}

/// A sentence-end separator undone at the end of the gap restores the
/// sentence context it cleared.
#[test]
fn remove_last_separator_restores_sentence_context() {
    let mut e = UltraFastViEngine::new();
    let mut h = Host::new();
    h.type_text(&mut e, "chaof");
    h.commit(&mut e, '!');
    assert!(e.remove_last_separator());
    h.screen.pop();
    h.commit(&mut e, ' ');
    h.type_text(&mut e, "dduwowcj");
    h.commit(&mut e, ' ');
    let text = e.sentence_text();
    assert!(text.contains("chào"), "{text:?}");

    // Control: with the '!' kept, "chào" ended the sentence and the walk
    // stops before it.
    let mut e = UltraFastViEngine::new();
    let mut h = Host::new();
    h.type_text(&mut e, "chaof");
    h.commit(&mut e, '!');
    h.commit(&mut e, ' ');
    h.type_text(&mut e, "dduwowcj");
    h.commit(&mut e, ' ');
    let text = e.sentence_text();
    assert!(text.contains("được"), "{text:?}");
    assert!(!text.contains("chào"), "{text:?}");
}
