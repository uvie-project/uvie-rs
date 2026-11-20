//! Sentence-level language context, the host's language hint, and the
//! double-press cancel collapse.
//!
//! Three features meet here:
//!
//! 1. a modifier-key cancel ("aww", "aa" after `â`) leaves no trace: every
//!    rebuild from the raw keystrokes drops the consumed key ("awware" →
//!    "aware", "newws" → "news"), while a doubled tone key stays ("off",
//!    "diffed", "mass" are genuine English spellings);
//! 2. the sentence remembers that it already contains a Vietnamese word, and
//!    an ambiguous word in such a sentence keeps its Vietnamese reading;
//! 3. the host's `ContextHint` (a NaturalLanguage verdict over
//!    `sentence_text`) overrides that for the sentence it describes.
//!
//! The harness mirrors the real dispatcher: `feed_diff` per character,
//! `commit_with_boundary_diff(boundary)` on a word break, the boundary char
//! posted natively and reported through `note_separator(boundary)`.

use uvie::diff::{ContextHint, Diffable};
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
        let mut engine = UltraFastViEngine::new();
        engine.set_input_method(InputMethod::Telex);
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
    host.type_seq(input)
}

#[test]
fn cancel_pair_collapses_to_one_literal_key() {
    // A doubled modifier is the cancel sequence: the render and the commit
    // must both drop the consumed key, exactly like a doubled tone key.
    for (input, expected) in [
        ("awware ", "aware "),
        ("awwa", "awa"),
        ("awwar ", "awar "),
        ("newws ", "news "),
        ("aww ", "aw "),
        ("aww", "aw"),
        ("awwa ", "awa "),
        // A doubled tone key already collapsed; the cancel must survive the
        // new mechanism unchanged.
        ("carr ", "car "),
        ("rassa ", "rasa "),
        ("tesst ", "test "),
        ("resset ", "reset "),
        ("masster ", "master "),
        ("hoss ", "hoss "),
        // Words whose doubled key IS the spelling keep every key.
        ("off ", "off "),
        ("diff ", "diff "),
        ("diffed ", "diffed "),
        ("coffee ", "coffee "),
        ("staff ", "staff "),
        ("lossless ", "lossless "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn cancel_collapse_keeps_vietnamese_typing() {
    for (input, expected) in [
        // The doubled modifier keys that already worked.
        ("uoww ", "uow "),
        ("showw ", "show "),
        ("aaww ", "aaww "),
        ("eeww ", "eeww "),
        ("ww ", "w "),
        ("www ", "ww "),
        // A `w` the pipeline swallowed for another reason is not a cancel.
        ("browserclaw ", "browserclaw "),
        ("chuwongw ", "chương "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn sentence_context_resets_at_the_sentence_end() {
    // The first sentence is Vietnamese; the second one is judged on its own.
    assert_eq!(
        type_telex("Tooi ddi hocj. this is a test "),
        "Tôi đi học. this is a test "
    );
    // `!`, `?` and a newline end a sentence too.
    assert_eq!(
        type_telex("Tooi ddi hocj! this is a test "),
        "Tôi đi học! this is a test "
    );
    assert_eq!(
        type_telex("Tooi ddi hocj? this is a test "),
        "Tôi đi học? this is a test "
    );
    // A `.` glued between two letters is a code dot, not a sentence end.
    assert_eq!(type_telex("nos a.b nos "), "nó a.b nó ");
}

#[test]
fn vietnamese_sentence_prefers_vietnamese_readings() {
    for (input, expected) in [
        // "car" renders "cả" (a top particle): the Vietnamese sentence keeps
        // it, whatever the English rarity score says.
        (
            "Tooi thaays car nhaf ddeuf vui ",
            "Tôi thấy cả nhà đều vui ",
        ),
        // English tech words still restore; the Vietnamese particle does not.
        (
            "hoom nay deploy server nos bij loiox ",
            "hôm nay deploy server nó bị lỗi ",
        ),
        // A sentence with no Vietnamese word keeps today's rules.
        ("the best car ", "the best car "),
        ("this is a test ", "this is a test "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn english_hint_restores_ambiguous_words() {
    // "cos" renders "có", a top Vietnamese particle a Vietnamese sentence
    // keeps. The host's recognizer read the sentence as English: it restores.
    let mut host = Host::new();
    host.type_seq("em ");
    host.engine.set_context_hint(ContextHint::English);
    assert_eq!(host.type_seq("cos "), "em cos ");
    // The same word without the hint keeps the Vietnamese reading.
    assert_eq!(type_telex("em cos "), "em có ");
}

#[test]
fn vietnamese_hint_keeps_vietnamese_after_an_english_word() {
    // "the" is an English function word, so today's rules would restore "nos"
    // to "nos"; the recognizer says the sentence is Vietnamese.
    let mut host = Host::new();
    host.type_seq("the ");
    host.engine.set_context_hint(ContextHint::Vietnamese);
    assert_eq!(host.type_seq("nos "), "the nó ");
    // Without the hint the English function word wins.
    assert_eq!(type_telex("the nos "), "the nos ");
}

#[test]
fn hint_and_sentence_reset_at_a_sentence_boundary() {
    let mut host = Host::new();
    host.engine.set_context_hint(ContextHint::English);
    host.type_seq("Tooi ddi ");
    assert_eq!(host.engine.context_hint(), ContextHint::English);
    // The sentence ends: the verdict described it and must not outlive it.
    host.type_seq("hocj. ");
    assert_eq!(host.engine.context_hint(), ContextHint::Auto);
    assert_eq!(host.text, "Tôi đi học. ");
}

#[test]
fn sentence_text_lists_the_current_sentence() {
    let mut host = Host::new();
    host.type_seq("Tooi ddi hocj. xin ");
    // Only the words after the last boundary, with their separators.
    assert_eq!(&*host.engine.sentence_text(), "xin ");
    host.type_seq("chaof ");
    assert_eq!(&*host.engine.sentence_text(), "xin chào ");
    // The composing word is not part of the sentence yet.
    host.type_seq("cacs");
    assert_eq!(&*host.engine.sentence_text(), "xin chào ");
    // A newline starts a new sentence.
    host.word_break('\n');
    assert_eq!(&*host.engine.sentence_text(), "");
    host.type_seq("nos ");
    assert_eq!(&*host.engine.sentence_text(), "nó ");
    // The retroactive English fix rewrites the words it restored.
    let mut host = Host::new();
    host.type_seq("this is a test ");
    assert_eq!(&*host.engine.sentence_text(), "this is a test ");
}

#[test]
fn ffi_sentence_text_writes_the_sentence() {
    use std::ffi::c_char;
    let engine = uvie::ffi::uvie_engine_new();
    assert!(!engine.is_null());
    let mut out = [0 as c_char; 64];
    for ch in "Tooi ddi hocj. xin chaof ".chars() {
        let key = ch as c_char;
        if is_break(ch) {
            let _ = uvie::ffi::uvie_engine_commit_with(engine, key, out.as_mut_ptr(), out.len());
            uvie::ffi::uvie_engine_note_separator(engine, key);
        } else {
            let _ = uvie::ffi::uvie_engine_feed(engine, key, out.as_mut_ptr(), out.len());
        }
    }
    let mut buf = [0 as c_char; 128];
    let n = uvie::ffi::uvie_engine_sentence_text(engine, buf.as_mut_ptr(), buf.len());
    assert_eq!(n, "xin chào ".len());
    let bytes: Vec<u8> = buf[..n].iter().map(|&b| b as u8).collect();
    assert_eq!(String::from_utf8(bytes).unwrap(), "xin chào ");
    // A short buffer truncates at a char boundary instead of splitting one.
    let mut small = [0 as c_char; 6];
    let n = uvie::ffi::uvie_engine_sentence_text(engine, small.as_mut_ptr(), small.len());
    assert_eq!(n, 5);
    let bytes: Vec<u8> = small[..n].iter().map(|&b| b as u8).collect();
    assert_eq!(String::from_utf8(bytes).unwrap(), "xin c");
    // The hint setter is part of the same ABI surface.
    uvie::ffi::uvie_engine_set_context_hint(engine, 2);
    uvie::ffi::uvie_engine_free(engine);
}

#[test]
fn composing_never_shows_the_cancelled_key() {
    // The composing render (no word boundary typed) collapses like the commit.
    assert_eq!(type_telex("awwa"), "awa");
    assert_eq!(type_telex("neww"), "new");
    assert_eq!(type_telex("carr"), "car");
}
