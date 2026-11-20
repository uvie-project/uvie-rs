//! English-in-Vietnamese-text accuracy: the round-3 rules.
//!
//! The host model mirrors the real dispatcher: `feed_diff` per character,
//! `commit_with_boundary_diff(boundary)` on a word break, the boundary char
//! posted natively and reported through `note_separator(boundary)`.
//!
//! Covered here:
//! 1. a TOP Vietnamese syllable is only restored after a curated English
//!    function word (or inside a proven English sentence);
//! 2. always-English tech words ("test", "list", "host", "cost", "boost");
//! 3. no Vietnamese syllable starts with j, f or z ("Jira");
//! 4. camelCase / mixed-case identifiers ("DeepSeek");
//! 5. long words keep every key ("browserclaw", 30-char identifiers);
//! 6. doubled tone keys inside a real English word ("diffed", "diffs");
//! 7. URLs / paths show their literal keys ("/usr/share",
//!    "https://github.com/x/...");
//! 8. hyphen compounds follow the English side ("low-level");
//! 9. contractions restore the word before the apostrophe ("didn't").

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
        let mut engine = UltraFastViEngine::new();
        engine.set_input_method(InputMethod::Telex);
        Self {
            engine,
            text: String::new(),
            caret: 0,
        }
    }

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

fn type_telex_quick_start(input: &str) -> String {
    let mut host = Host::new();
    host.engine.set_quick_start(true);
    host.type_seq(input)
}

#[test]
fn top_syllable_needs_an_english_function_word() {
    for (input, expected) in [
        // "cả" is a top Vietnamese syllable; a function word restores it.
        ("the car ", "the car "),
        ("my car is red ", "my car is red "),
        ("I love this car ", "I love this car "),
        ("it is the best car ", "it is the best car "),
        // A dictionary word is not a function word: "có" stays Vietnamese,
        // with or without a comma in between.
        ("vaof main, cos ", "vào main, có "),
        ("vaof main cos ", "vào main có "),
        ("main cos ", "main có "),
        // Plain Vietnamese sentences never flip.
        ("em cos ", "em có "),
        ("cos ver ok ", "có vẻ ok "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn always_english_tech_words() {
    for (input, expected) in [
        // Rare Vietnamese readings of real tech words, whatever precedes them.
        ("ddang vieets test cho ", "đang viết test cho "),
        ("moojt list ", "một list "),
        ("hai host ", "hai host "),
        ("ba cost ", "ba cost "),
        ("boosts ", "boosts "),
        ("tests lists hosts costs ", "tests lists hosts costs "),
        // "bánh tét" is the Vietnamese dish: the reading stands after "bánh".
        ("banhs test ", "bánh tét "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn no_vietnamese_syllable_starts_with_j_f_z() {
    for (input, expected) in [
        ("Jira ", "Jira "),
        ("jira ", "jira "),
        ("jira-bug-status ", "jira-bug-status "),
        ("zoo ", "zoo "),
        ("Zalo ", "Zalo "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
    // Quick Start expands j to gi before the render, so the rule stays out of
    // its way (the expansion, not the raw letters, is what the user asked
    // for).
    assert_eq!(type_telex_quick_start("Jira "), "giiả ");
    assert_eq!(type_telex_quick_start("fan "), "phan ");
}

#[test]
fn mixed_case_identifiers_show_their_keys() {
    for (input, expected) in [
        ("DeepSeek ", "DeepSeek "),
        (
            "applyCompoundForwardDeletes ",
            "applyCompoundForwardDeletes ",
        ),
        (
            "resetEngineForLanguageSwitch ",
            "resetEngineForLanguageSwitch ",
        ),
        ("iPhone ", "iPhone "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn long_words_never_lose_keys() {
    for (input, expected) in [
        // The trailing `w` is a Telex modifier: the engine swallowed it, so
        // the render lost a letter. The word is English, not Vietnamese.
        ("browserclaw ", "browserclaw "),
        // A 30-key identifier outgrows the 24-keystroke raw buffer; the
        // prefix commits mid-word and the tail must still land.
        (
            ".resetEngineForLanguageSwitch ",
            ".resetEngineForLanguageSwitch ",
        ),
        (
            "zzabcdefghijklmnopqrstuvwxyz ",
            "zzabcdefghijklmnopqrstuvwxyz ",
        ),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn doubled_tone_key_english_words_with_suffixes() {
    for (input, expected) in [
        ("diffed ", "diffed "),
        ("diffs ", "diffs "),
        ("diffing ", "diffing "),
        ("offs ", "offs "),
        ("passed ", "passed "),
        // A cancel render the user asked for keeps its text.
        ("carr ", "car "),
        ("hoss ", "hoss "),
        ("rassa ", "rasa "),
        // The Telex cancel still works inside Vietnamese words.
        ("masster ", "master "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn urls_and_paths_show_their_literal_keys() {
    for (input, expected) in [
        ("/usr/share ", "/usr/share "),
        ("/usr/lib/fcitx5/libuvie.so ", "/usr/lib/fcitx5/libuvie.so "),
        ("/rustc/<40-hex>/ ", "/rustc/<40-hex>/ "),
        (
            "https://github.com/x/blob/main/docs/a.md ",
            "https://github.com/x/blob/main/docs/a.md ",
        ),
        (
            "(`/usr/lib/fcitx5/libuvie.so`) ",
            "(`/usr/lib/fcitx5/libuvie.so`) ",
        ),
        (
            "[signed-by=/usr/share/keyrings/uvie.gpg] ",
            "[signed-by=/usr/share/keyrings/uvie.gpg] ",
        ),
        ("~/uvie/docs ", "~/uvie/docs "),
        ("github.com/thuupx/uvie-rs ", "github.com/thuupx/uvie-rs "),
        // A slash inside Vietnamese prose is not a path.
        ("tieengs/vieetj ", "tiếng/việt "),
        ("tieengs, vieetj ", "tiếng, việt "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn hyphen_compounds_follow_their_english_side() {
    for (input, expected) in [
        ("low-level ", "low-level "),
        ("backspace-arms ", "backspace-arms "),
        // Vietnamese on both sides keeps its marks.
        ("tieengs-vieetj ", "tiếng-việt "),
        ("hocj-sinh ", "học-sinh "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn contractions_restore_the_word_before_the_apostrophe() {
    for (input, expected) in [
        ("didn't ", "didn't "),
        ("it's ", "it's "),
        ("I'm ", "I'm "),
        ("don't ", "don't "),
        ("doesn't ", "doesn't "),
        ("isn't ", "isn't "),
        ("we'll ", "we'll "),
        ("they're ", "they're "),
        ("I've ", "I've "),
        ("I'd ", "I'd "),
        // A lone apostrophe (a closing quote) restores nothing.
        ("'hocj' ", "'học' "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn english_content_words_do_not_flip_vietnamese_particles() {
    // Vietnamese developers mix English nouns and verbs into Vietnamese
    // sentences: two English content words in a row are no proof that the
    // next top syllable is English. A function word is ("the best car").
    for (input, expected) in [
        ("deploy server nos bij loiox ", "deploy server nó bị lỗi "),
        ("test case cos vaasn ddeef ", "test case có vấn đề "),
        ("build fail car ", "build fail cả "),
        ("awn no car nhaf ", "ăn no cả nhà "),
        ("so sasnh car hai ", "so sánh cả hai "),
        ("it is the best car ", "it is the best car "),
        (
            "xem https://dduowngf.com/abc ",
            "xem https://dduowngf.com/abc ",
        ),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}
