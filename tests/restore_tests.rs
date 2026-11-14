//! Word-boundary "restore raw keystrokes" (LabanKey-style).
//!
//! A finished token that is not a valid Vietnamese word must show the literal
//! keystrokes instead of the marks the pipeline applied to it ("law " must not
//! become "lă "). The host model mirrors the macOS EventTap: `feed_diff` per
//! character, `commit_diff` on Space (apply backspaces + suffix to the screen,
//! then post the space itself).

use uvie::diff::Diffable;
use uvie::{InputMethod, UltraFastViEngine};

/// Minimal host: the on-screen text, the caret, and the engine.
struct Host {
    engine: UltraFastViEngine,
    text: String,
    caret: usize,
    /// Absolute offset of the newest text the engine owns (the anchor).
    anchor: usize,
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
            anchor: 0,
        }
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
            self.anchor = self.caret - 1;
            // The space is posted natively: report it for the caret geometry.
            self.engine.note_separator();
        }
        self.text.clone()
    }

    fn arrow_left(&mut self) {
        self.caret = self.caret.saturating_sub(1);
    }

    /// The caret's distance back to the anchor (what the host passes to
    /// `edit_at_caret_diff`); 0 = the caret at the newest word's end.
    fn caret_back(&self) -> usize {
        self.anchor.saturating_sub(self.caret)
    }

    /// Re-enter the newest committed word with `ch` appended to its raw
    /// keystrokes.
    fn edit(&mut self, ch: char) -> Option<String> {
        let (bs, fwd, out) = self.engine.edit_at_caret_diff(self.caret_back(), ch)?;
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

fn type_telex(input: &str) -> String {
    let mut host = Host::new(InputMethod::Telex);
    host.type_seq(input)
}

fn type_vni(input: &str) -> String {
    let mut host = Host::new(InputMethod::Vni);
    host.type_seq(input)
}

#[test]
fn restores_non_vietnamese_words() {
    for (input, expected) in [
        ("law ", "law "),
        ("saw ", "saw "),
        ("wow ", "wow "),
        ("dawf ", "dawf "),
        ("after ", "after "),
        ("water ", "water "),
        ("server ", "server "),
        ("laravel ", "laravel "),
        ("display ", "display "),
        ("master ", "master "),
        // Case is preserved (the render is analysed lowercase).
        ("Law ", "Law "),
        ("LAW ", "LAW "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn keeps_vietnamese_words() {
    for (input, expected) in [
        ("tieengs Vieetj ", "tiếng Việt "),
        ("nguowif ", "người "),
        ("thuowr ", "thuở "),
        ("huow ", "huơ "),
        // Teen-code abbreviations are not syllables and are never restored.
        ("ddc ", "đc "),
        ("ddt ", "đt "),
        // No tone yet: the user may add it later via post-commit editing.
        ("vieet ", "viêt "),
        ("ows ", "ớ "),
        ("uwf ", "ừ "),
        ("owi ", "ơi "),
        ("ura ", "ủa "),
        ("DDawsk Lawks ", "Đắk Lắk "),
        // Plain-letter differences are intentional double-key cancels.
        ("masster ", "master "),
        ("off ", "of "),
        ("hoaf ", "hoà "),
        ("khuyeens ", "khuyến "),
        ("giaf ", "già "),
        ("quaf ", "quà "),
    ] {
        assert_eq!(type_telex(input), expected, "input: {input:?}");
    }
}

#[test]
fn vni_restores_without_telex_specifics() {
    for (input, expected) in [
        // 'w' is a plain letter in VNI: nothing to restore.
        ("law ", "law "),
        // Open ă is an always-closed rhyme: the digits come back.
        ("a8 ", "a8 "),
        ("tie6ng1 ", "tiếng "),
    ] {
        assert_eq!(type_vni(input), expected, "input: {input:?}");
    }
}

#[test]
fn toggle_off_keeps_the_vietnamese_render() {
    let mut host = Host::new(InputMethod::Telex);
    host.engine.set_english_override(false);
    assert_eq!(host.type_seq("law "), "lă ");
}

#[test]
fn edit_after_restore_rerenders_from_raw() {
    let mut host = Host::new(InputMethod::Telex);
    assert_eq!(host.type_seq("dawf "), "dawf ");
    host.arrow_left(); // caret onto the committed word's end
    assert_eq!(host.caret_back(), 0);
    let edited = host
        .edit('n')
        .expect("editing a restored word must be handled");
    // The stored raw is "dawf", so raw + 'n' re-renders as "dằn".
    assert_eq!(edited, "dằn ");
}
