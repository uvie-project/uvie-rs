//! Re-opening a dictionary-hijacked word as Vietnamese.
//!
//! The English dictionary override fires per keystroke: once `word_raw` is a
//! dictionary word the raw word is committed and the rest of the word passes
//! through raw. When that dictionary word is a prefix of a common Vietnamese
//! keystroke sequence, Vietnamese became untypeable ("soon"+"g" → "soong"
//! instead of "sông"). Once the extended word can no longer become an English
//! word and renders as one complete Vietnamese syllable, the word is
//! re-opened as Vietnamese.
//!
//! The host model mirrors the macOS EventTap: `feed_diff` per character,
//! `commit_diff` on Space (apply backspaces + suffix to the screen, then post
//! the space itself). Modern orthography is left at the engine default (off).

use uvie::diff::Diffable;
use uvie::{InputMethod, UltraFastViEngine};

/// Minimal host: the on-screen text, the caret, and the engine.
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

    /// Type `input`; a space commits the word and is then posted natively.
    fn type_seq(&mut self, input: &str) -> String {
        for ch in input.chars() {
            if ch != ' ' {
                self.feed(ch);
                continue;
            }
            let (bs, out) = self.engine.commit_diff();
            let out = out.to_string();
            self.apply(bs, &out);
            let head: String = self.text.chars().take(self.caret).collect();
            let tail: String = self.text.chars().skip(self.caret).collect();
            self.text = format!("{head} {tail}");
            self.caret += 1;
        }
        self.text.clone()
    }

    fn backspace(&mut self) {
        let (bs, out) = self.engine.backspace_diff();
        let out = out.to_string();
        self.apply(bs, &out);
    }
}

fn type_telex(input: &str) -> String {
    Host::new().type_seq(input)
}

#[test]
fn reopens_dictionary_prefixes_as_vietnamese() {
    for (input, expected) in [
        ("soong ", "sông "),
        ("soongs ", "sống "),
        ("meetj ", "mệt "),
        ("lawsm ", "lắm "),
        ("lawsng ", "lắng "),
        ("DDawsk Lawsk ", "Đắk Lắk "),
        ("gaasp ", "gấp "),
        ("bootj ", "bột "),
        ("buyst ", "buýt "),
        ("Soong ", "Sông "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn keeps_english_words() {
    for (input, expected) in [
        ("soon ", "soon "),
        ("sooner ", "sooner "),
        ("meet ", "meet "),
        ("meeting ", "meeting "),
        ("laws ", "laws "),
        ("lawsuit ", "lawsuit "),
        ("boot ", "boot "),
        ("keep ", "keep "),
        ("keeping ", "keeping "),
        ("master ", "master "),
        ("masster ", "master "),
        ("passkey ", "passkey "),
        ("massive ", "massive "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn backspace_after_reopen_walks_the_word() {
    let mut host = Host::new();
    assert_eq!(host.type_seq("soong"), "sông");
    host.backspace();
    // The word is composing again, so backspace pops the last keystroke.
    assert_eq!(host.text, "sôn");
}

#[test]
fn reports_trade_offs() {
    // No assertion: the expected trade-off is Vietnamese whenever the render
    // is a valid syllable and the extended word is not in the dictionary.
    for input in ["boots ", "roots ", "loops ", "keeps ", "meets "] {
        println!("{input:?} -> {:?}", type_telex(input));
    }
}
