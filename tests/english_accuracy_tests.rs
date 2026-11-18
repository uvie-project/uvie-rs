//! English-accuracy rules at word commit.
//!
//! Five rules sit on top of the dictionary / restore-raw machinery, all gated
//! by `set_english_override`:
//!
//! 1. ambiguous words — a common English word whose Vietnamese render is
//!    rarely used ("test" → "tét", "or" → "ỏ") is English; the previous
//!    committed word decides the borderline ones;
//! 2. retroactive fix — an English word restores the ambiguous words in front
//!    of it ("this is a test" must not stay "thí í a tét");
//! 3. doubled Telex tone keys — "off"/"diff"/"lossless" are English words,
//!    not the cancel sequence ("masster" → "master" still cancels);
//! 4. ALL-CAPS acronyms — "JIRA"/"AX"/"DESC" whose render is a rare syllable
//!    keep their letters ("VIEETJ" → "VIỆT" stays Vietnamese);
//! 5. code identifiers — a word glued to a code char ("feed_diff", "a.b") or
//!    ended by one ("raw_chars") shows its literal keystrokes.
//!
//! The host model mirrors the real dispatcher: `feed_diff` per character,
//! `commit_with_boundary_diff(boundary)` on a word break, the boundary char
//! posted natively and reported through `note_separator(boundary)`.

use uvie::diff::Diffable;
use uvie::{InputMethod, UltraFastViEngine};

/// Punctuation that ends a word (the host's break table).
fn is_break(ch: char) -> bool {
    matches!(
        ch,
        ' ' | '\t'
            | '\n'
            | '\r'
            | '.'
            | ','
            | ';'
            | ':'
            | '?'
            | '!'
            | '\''
            | '"'
            | '('
            | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '<'
            | '>'
            | '/'
            | '\\'
            | '|'
            | '-'
            | '_'
            | '='
            | '+'
            | '*'
            | '&'
            | '^'
            | '%'
            | '$'
            | '#'
            | '@'
            | '~'
            | '`'
    )
}

/// Minimal host: the on-screen text, the caret and the separator bookkeeping.
struct Host {
    engine: UltraFastViEngine,
    text: String,
    caret: usize,
}

impl Host {
    fn new() -> Self {
        Self {
            engine: UltraFastViEngine::new(),
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

    /// Commit the composing word at `boundary`, then post the char natively.
    fn word_break(&mut self, boundary: char) {
        let (bs, out) = self.engine.commit_with_boundary_diff(boundary);
        let out = out.to_string();
        self.apply(bs, &out);
        let head: String = self.text.chars().take(self.caret).collect();
        let tail: String = self.text.chars().skip(self.caret).collect();
        self.text = format!("{head}{boundary}{tail}");
        self.caret += 1;
        self.engine.note_separator(boundary);
    }

    fn type_seq(&mut self, input: &str) -> String {
        for ch in input.chars() {
            if is_break(ch) {
                self.word_break(ch);
            } else {
                self.feed(ch);
            }
        }
        self.text.clone()
    }
}

fn type_telex(input: &str) -> String {
    let mut host = Host::new();
    host.engine.set_input_method(InputMethod::Telex);
    host.type_seq(input)
}

#[test]
fn ambiguous_words_are_english_in_english_sentences() {
    for (input, expected) in [
        // "test"/"best"/"see" render as real Vietnamese syllables alone.
        ("this is a test ", "this is a test "),
        ("as soon as possible ", "as soon as possible "),
        ("if you want to use it ", "if you want to use it "),
        ("I think it is a test ", "I think it is a test "),
        ("The cost of the host ", "The cost of the host "),
        // A rare Vietnamese reading is English everywhere ("ỏ" is unused).
        ("or ", "or "),
        ("of the file ", "of the file "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn vietnamese_sentences_keep_their_marks() {
    for (input, expected) in [
        ("cacs this sinh ", "các thí sinh "),
        ("nos how ", "nó hơ "),
        ("ddaay how ", "đây hơ "),
        ("than this ", "than thí "),
        ("hocj moon ", "học môn "),
        ("lats nuwax ", "lát nữa "),
        ("owr nhaf ", "ở nhà "),
        // A loanword mixed into a Vietnamese sentence keeps it Vietnamese.
        ("ok rets ", "ok rét "),
        ("file lats ", "file lát "),
        // A lone Vietnamese word is never flipped by the ambiguous rule.
        ("bít ", "bít "),
        ("cháo ", "cháo "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn retroactive_fix_restores_older_english_words() {
    // "this"/"is" render "thí"/"í"; the dictionary word "test" proves the
    // sentence is English, so the earlier words are restored too.
    assert_eq!(type_telex("this is a test "), "this is a test ");
    // The walk stops at the first word that is not English: a Vietnamese
    // word before the English one keeps its marks.
    assert_eq!(type_telex("nos this is a test "), "nó this is a test ");
    // At most 3 older words are rewritten.
    assert_eq!(type_telex("this is a real test "), "this is a real test ");
}

#[test]
fn doubled_tone_keys_in_english_words() {
    for (input, expected) in [
        // The second key is the Telex cancel, but the raw is an English word.
        ("off ", "off "),
        ("diff ", "diff "),
        ("lossless ", "lossless "),
        (
            "a diff of the lossless pass ",
            "a diff of the lossless pass ",
        ),
        // Not English: the double key stays a cancel.
        ("masster ", "master "),
        // Nothing to restore: the render is already literal.
        ("class ", "class "),
        ("process ", "process "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn all_caps_acronyms_keep_their_letters() {
    for (input, expected) in [
        ("JIRA ", "JIRA "),
        ("AX ", "AX "),
        ("OS ", "OS "),
        ("DIR ", "DIR "),
        ("DESC ", "DESC "),
        // A common syllable keeps the acronym Vietnamese.
        ("VIEETJ NAM ", "VIỆT NAM "),
        // Not all caps: the ambiguous / dictionary rules decide those.
        ("the best ", "the best "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn code_identifiers_show_their_literal_keys() {
    for (input, expected) in [
        ("`feed_diff` ", "`feed_diff` "),
        ("`raw_chars` ", "`raw_chars` "),
        ("UVIE_RS_DIR ", "UVIE_RS_DIR "),
        ("try_apply_w_non_cancel ", "try_apply_w_non_cancel "),
        ("uvie_engine_commit_with ", "uvie_engine_commit_with "),
        ("a.b ", "a.b "),
        ("x=1 ", "x=1 "),
        // Glued to text on its left; a leading `@` has no left neighbour.
        ("hoas,@hoas ", "hóa,@hoas "),
        ("uvie-host ", "uvie-host "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn vietnamese_punctuation_is_not_code() {
    for (input, expected) in [
        // `(`, `/`, `-`, `:`, `,` and a terminating `.` are normal prose.
        ("tieengs, ", "tiếng, "),
        ("(tieengs) ", "(tiếng) "),
        ("tieengs/vieetj ", "tiếng/việt "),
        ("tieengs-vieetj ", "tiếng-việt "),
        ("tieengs: ", "tiếng: "),
        ("tieengs. ", "tiếng. "),
        ("nguowif ", "người "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn toggle_off_keeps_the_vietnamese_render() {
    let mut host = Host::new();
    host.engine.set_english_override(false);
    assert_eq!(host.type_seq("this is a test "), "thí í a tét ");
    let mut host = Host::new();
    host.engine.set_english_override(false);
    assert_eq!(host.type_seq("off "), "of ");
}
