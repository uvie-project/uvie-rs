//! A tone key cancelled by its double press is a LITERAL consonant for the
//! rest of the word: it must never be re-read as a tone key and must not let
//! the vowel pair around it merge.
//!
//! User report: "resset" (ss = "cancel the sắc, I want a literal s") produced
//! "rết" instead of "reset". The double-tone cancel reverted the tone and
//! pushed the cancelling key as a plain consonant, so the later doubled vowel
//! (`e…s…e`, `o…f…o`, `a…x…a`) treated the literal key as a tone key again and
//! merged the pair into a circumflex nucleus with that tone.

use uvie::diff::Diffable;
use uvie::{InputMethod, UltraFastViEngine};

/// Apply a (backspaces, suffix) instruction to the screen model.
fn apply(screen: &mut String, bs: usize, suffix: &str) {
    let keep = screen.chars().count().saturating_sub(bs);
    let mut out: String = screen.chars().take(keep).collect();
    out.push_str(suffix);
    *screen = out;
}

/// Type `input` through the diff API and return the on-screen text.
/// A space commits the composing word (the boundary char is appended).
fn type_diff(input: &str, method: InputMethod) -> String {
    let mut engine = UltraFastViEngine::new();
    engine.set_input_method(method);
    let mut screen = String::new();
    for ch in input.chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        let suffix = suffix.to_string();
        apply(&mut screen, bs, &suffix);
    }
    screen
}

/// Type `input`, press Backspace `backspaces` times, then type `retype`.
fn type_backspace_retype(
    input: &str,
    backspaces: usize,
    retype: &str,
    method: InputMethod,
) -> String {
    let mut engine = UltraFastViEngine::new();
    engine.set_input_method(method);
    let mut screen = String::new();
    for ch in input.chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        let suffix = suffix.to_string();
        apply(&mut screen, bs, &suffix);
    }
    for _ in 0..backspaces {
        let (bs, suffix) = engine.backspace_diff();
        let suffix = suffix.to_string();
        apply(&mut screen, bs, &suffix);
    }
    for ch in retype.chars() {
        let (bs, suffix) = engine.feed_diff(ch);
        let suffix = suffix.to_string();
        apply(&mut screen, bs, &suffix);
    }
    screen
}

fn assert_cases(cases: &[(&str, &str)], method: InputMethod) {
    for &(input, want) in cases {
        assert_eq!(
            type_diff(input, method),
            want,
            "input {input:?} must render {want:?}"
        );
    }
}

// ------------------------------------------------------------------
// The reported bug: the cancelled key comes back as a tone key
// ------------------------------------------------------------------

#[test]
fn cancelled_tone_key_is_literal() {
    assert_cases(
        &[
            // "resse" composes as "rese" (the second s cancelled the sắc).
            ("resse", "rese"),
            ("resset ", "reset "),
            ("toffo ", "tofo "),
            ("berre ", "bere "),
        ],
        InputMethod::Telex,
    );
}

#[test]
fn cancelled_tone_key_is_literal_vni() {
    assert_cases(
        &[
            ("re11e", "re1e"),
            ("to22o", "to2o"),
            ("ra11a ", "ra11a "),
            // The cancelled word commits as its render, digit included.
            ("ca33 ", "ca3 "),
        ],
        InputMethod::Vni,
    );
}

// ------------------------------------------------------------------
// Words that already worked must keep working
// ------------------------------------------------------------------

#[test]
fn cancel_words_keep_their_current_render() {
    assert_cases(
        &[
            ("tesst ", "test "),
            ("masster ", "master "),
            ("ressult ", "result "),
            ("dosse ", "dose "),
            ("rassa ", "rasa "),
            ("taxxa ", "taxa "),
            ("ressolve ", "resolve "),
            ("possess ", "possess "),
            ("assess ", "assess "),
            ("classes ", "classes "),
            ("dessert ", "dessert "),
        ],
        InputMethod::Telex,
    );
}

// ------------------------------------------------------------------
// Normal Telex stays intact: a tone key BETWEEN a vowel pair is legit
// when it was NOT cancelled
// ------------------------------------------------------------------

#[test]
fn normal_mid_nucleus_tones_still_apply() {
    assert_cases(
        &[
            ("befe ", "bề "),
            ("safam ", "sầm "),
            ("tieengs ", "tiếng "),
            ("vieetj ", "việt "),
            // "tofot": the f is a real tone key between the o halves.
            ("tofot", "tồt"),
        ],
        InputMethod::Telex,
    );
}

#[test]
fn normal_mid_nucleus_tones_still_apply_vni() {
    // "to2o" = t + o + huyền + o: a mid-nucleus tone, not a cancel.
    assert_cases(&[("to2o ", "tồ ")], InputMethod::Vni);
}

// ------------------------------------------------------------------
// Backspace walks the cancelled word back like any composing word
// ------------------------------------------------------------------

#[test]
fn backspace_walks_back_through_the_cancel() {
    // "resset" composes as "reset": erasing the 't' leaves "rese", then the
    // 'e' leaves "res" (the collapsed cancel form).
    assert_eq!(
        type_backspace_retype("resset", 1, "", InputMethod::Telex),
        "rese"
    );
    assert_eq!(
        type_backspace_retype("resset", 2, "", InputMethod::Telex),
        "res"
    );
    // Retyping the erased keys must not resurrect the tone: the snapshot
    // keeps the cancelled key literal.
    assert_eq!(
        type_backspace_retype("resset", 2, "et", InputMethod::Telex),
        "reset"
    );
    // Erasing the whole word leaves nothing behind.
    assert_eq!(
        type_backspace_retype("resset", 6, "", InputMethod::Telex),
        ""
    );
    // ... and typing it again from scratch renders the same word.
    assert_eq!(
        type_backspace_retype("resset", 6, "resset", InputMethod::Telex),
        "reset"
    );
    // VNI: "re11e" composes as "re1e".
    assert_eq!(
        type_backspace_retype("re11e", 1, "", InputMethod::Vni),
        "re1"
    );
    assert_eq!(
        type_backspace_retype("re11e", 1, "e", InputMethod::Vni),
        "re1e"
    );
}

// ------------------------------------------------------------------
// The English sticky fallback ("masster") keeps its cancel
// ------------------------------------------------------------------

#[test]
fn english_fallback_still_cancels() {
    assert_eq!(
        type_diff("masster ", InputMethod::Telex),
        "master ",
        "the dictionary fallback must keep the double-s cancel"
    );
}

// ------------------------------------------------------------------
// Post-commit editing replays the raw keystrokes, cancel included
// ------------------------------------------------------------------

/// Screen + caret model for the post-commit edit contract.
struct EditScreen {
    text: String,
    caret: usize,
    anchor: usize,
    engine: UltraFastViEngine,
}

impl EditScreen {
    fn new() -> Self {
        Self {
            text: String::new(),
            caret: 0,
            anchor: 0,
            engine: UltraFastViEngine::new(),
        }
    }

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

    /// Type `word` and commit it; the host posts the space natively.
    fn commit_word(&mut self, word: &str) {
        for ch in word.chars() {
            self.feed(ch);
        }
        let (bs, out) = self.engine.commit_diff();
        let out = out.to_string();
        self.apply(bs, &out);
        let head: String = self.text.chars().take(self.caret).collect();
        let tail: String = self.text.chars().skip(self.caret).collect();
        self.text = format!("{head} {tail}");
        self.caret += 1;
        self.anchor = self.caret - 1;
        self.engine.note_separator(' ');
    }

    /// Re-enter the newest committed word with `ch` appended to its raw.
    fn edit_newest(&mut self, ch: char) -> Option<String> {
        let caret_back = self.anchor.saturating_sub(self.caret);
        let (bs, fwd, out) = self.engine.edit_at_caret_diff(caret_back, ch)?;
        let out = out.to_string();
        let kept = self.caret.saturating_sub(bs);
        let head: String = self.text.chars().take(kept).collect();
        let tail: String = self.text.chars().skip(self.caret + fwd).collect();
        self.text = format!("{head}{out}{tail}");
        self.caret = kept + out.chars().count();
        self.anchor = self.caret;
        Some(self.text.clone())
    }
}

/// Fresh-typing oracle for the edit contract: `word` + `ch`, committed.
fn type_fresh(word: &str, ch: char) -> String {
    let mut s = EditScreen::new();
    for c in format!("{word}{ch}").chars() {
        s.feed(c);
    }
    let _ = s.engine.commit_diff();
    s.text.push(' ');
    s.text
}

#[test]
fn edit_at_replays_the_cancelled_key_as_literal() {
    for (word, ch) in [
        ("resset", 's'),
        ("toffo", 'j'),
        ("berre", 'x'),
        ("rassa", 's'),
        ("taxxa", 'f'),
    ] {
        let fresh = type_fresh(word, ch);
        let mut s = EditScreen::new();
        s.commit_word(word);
        // The committed word is on screen; step the caret onto its end.
        s.caret -= 1;
        let edited = s
            .edit_newest(ch)
            .unwrap_or_else(|| panic!("edit of {word:?} with {ch:?} was not handled"));
        assert_eq!(
            edited, fresh,
            "editing {word:?} with {ch:?} must equal fresh typing"
        );
    }
}

// ------------------------------------------------------------------
// The cancel survives a V-C-V split / word boundary in a sentence
// ------------------------------------------------------------------

#[test]
fn cancel_inside_a_sentence() {
    assert_eq!(type_diff("resset nam ", InputMethod::Telex), "reset nam ");
    assert_eq!(type_diff("rassa taxxa ", InputMethod::Telex), "rasa taxa ");
}

#[test]
fn cancel_keeps_letter_case() {
    // The literal key and the restored keystrokes keep the typed case.
    assert_cases(
        &[
            ("Rassa ", "Rasa "),
            ("RASSA ", "RASA "),
            ("Resset ", "Reset "),
        ],
        InputMethod::Telex,
    );
}
