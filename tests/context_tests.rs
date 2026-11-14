//! Sentence-context English preference at word commit.
//!
//! A common English word that renders as a valid Vietnamese syllable in
//! isolation ("how" → "hơ", "best" → "bét") is restored to its raw keystrokes
//! when the PREVIOUS committed word is clearly English ("Hello, how are
//! you?"). Vietnamese context must never trigger it. The host model mirrors
//! the macOS EventTap: `feed_diff` per character, `commit_diff` on a word
//! boundary (space/punctuation), then the boundary char is posted natively.

use uvie::diff::Diffable;
use uvie::{InputMethod, UltraFastViEngine};

/// Minimal host: the on-screen text and the caret.
struct Host {
    engine: UltraFastViEngine,
    text: String,
    caret: usize,
}

impl Host {
    fn new(method: InputMethod) -> Self {
        let mut engine = UltraFastViEngine::new();
        engine.set_input_method(method);
        engine.set_modern_orthography(true);
        Self {
            engine,
            text: String::new(),
            caret: 0,
        }
    }

    /// Apply a (backspaces, suffix) instruction at the caret.
    fn apply(&mut self, bs: usize, out: &str) {
        let kept = self.caret.saturating_sub(bs);
        let head: String = self.text.chars().take(kept).collect();
        let tail: String = self.text.chars().skip(self.caret).collect();
        self.text = format!("{head}{out}{tail}");
        self.caret = kept + out.chars().count();
    }

    fn feed(&mut self, ch: char) {
        let (bs, out) = self.engine.feed_diff(ch);
        let out = out.to_string();
        self.apply(bs, &out);
    }

    /// Commit the composing word, then post the boundary char natively.
    fn commit_and_post(&mut self, ch: char) {
        let (bs, out) = self.engine.commit_diff();
        let out = out.to_string();
        self.apply(bs, &out);
        let head: String = self.text.chars().take(self.caret).collect();
        let tail: String = self.text.chars().skip(self.caret).collect();
        self.text = format!("{head}{ch}{tail}");
        self.caret += 1;
    }

    fn type_seq(&mut self, input: &str) -> String {
        for ch in input.chars() {
            if ch == ' ' || ch == ',' || ch == '?' || ch == '.' {
                self.commit_and_post(ch);
            } else {
                self.feed(ch);
            }
        }
        self.text.clone()
    }
}

fn type_telex(input: &str) -> String {
    let mut host = Host::new(InputMethod::Telex);
    host.type_seq(input)
}

#[test]
fn english_context_restores_ambiguous_words() {
    for (input, expected) in [
        // "hello" is an English context word; "how" renders "hơ" alone.
        ("Hello, how are you?", "Hello, how are you?"),
        // "you"/"know"/"it"/"works" pass through unchanged; "how" is
        // restored because "know" is English context.
        ("you know how it works ", "you know how it works "),
        ("it is here ", "it is here "),
        ("and here we go ", "and here we go "),
        // "soon" is restored by the dictionary, so the second "as" sees an
        // English previous word. The first "as" has no context.
        ("as soon as possible ", "á soon as possible "),
        // Case is preserved.
        ("Hello, HOW are you?", "Hello, HOW are you?"),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn first_word_has_no_context() {
    assert_eq!(type_telex("how are you? "), "hơ are you? ");
}

#[test]
fn vietnamese_context_is_unaffected() {
    for (input, expected) in [
        ("trowif rets ", "trời rét "),
        ("lats nuwax ", "lát nữa "),
        ("hocj moon ", "học môn "),
        ("cacs this sinh ", "các thí sinh "),
        // A Vietnamese previous word never flips the context, so the
        // ambiguous word keeps its Vietnamese marks.
        ("nos how ", "nó hơ "),
        ("ddaay how ", "đây hơ "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn loanword_context_does_not_trigger() {
    for (input, expected) in [
        ("ok how ", "ok hơ "),
        ("file how ", "file hơ "),
        ("file lats ", "file lát "),
        ("ok rets ", "ok rét "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn forced_context_words_and_vietnamese_tokens() {
    // "hi" and "the" are rare Vietnamese tokens forced into the context
    // table; "than" (than đá) is a real Vietnamese word and stays out.
    assert_eq!(type_telex("Hi, how are you?"), "Hi, how are you?");
    assert_eq!(type_telex("the best way "), "the best way ");
    assert_eq!(type_telex("than this "), "than thí ");
    // Chaining: a word restored by the context rule is context too.
    assert_eq!(type_telex("what is best "), "what is best ");
}

#[test]
fn toggle_off_keeps_the_vietnamese_render() {
    let mut host = Host::new(InputMethod::Telex);
    host.engine.set_english_override(false);
    assert_eq!(host.type_seq("Hello, how are you?"), "Hello, hơ are you?");
}
