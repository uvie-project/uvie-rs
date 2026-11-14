//! Caret-aware screen simulator over the real `Dispatcher`.
//!
//! Script chars: `<` Backspace, `{` Left, `}` Right, `|` in the output marks
//! the caret. Run with `cargo run --release -p uvie-host --example zz_sim`.

use uvie::InputMethod;
use uvie_host::dispatcher::{Context, Dispatch, Dispatcher, InputAction};
use uvie_host::engine_session::{EngineOptions, EngineSession};
use uvie_host::keys::KeyEvent;

struct Sim {
    d: Dispatcher,
    s: Vec<char>,
    c: usize,
}
impl Sim {
    fn new() -> Self {
        let d = Dispatcher::new(EngineSession::with_options(
            InputMethod::Telex,
            EngineOptions {
                english_override: true,
                ..Default::default()
            },
        ));
        Sim { d, s: vec![], c: 0 }
    }
    fn plan(&mut self, p: &[InputAction]) {
        for a in p {
            match a {
                InputAction::Backspace(n) => {
                    for _ in 0..*n {
                        if self.c > 0 {
                            self.c -= 1;
                            self.s.remove(self.c);
                        }
                    }
                }
                InputAction::ForwardDelete(n) => {
                    for _ in 0..*n {
                        if self.c < self.s.len() {
                            self.s.remove(self.c);
                        }
                    }
                }
                InputAction::Text(t) => {
                    for ch in t.chars() {
                        self.s.insert(self.c, ch);
                        self.c += 1;
                    }
                }
                #[allow(unreachable_patterns)]
                _ => {}
            }
        }
    }
    fn key(&mut self, k: KeyEvent, native: impl FnOnce(&mut Sim)) {
        let ctx = Context {
            language: None,
            app_excluded: false,
            non_latin_layout: false,
        };
        match self.d.handle(k, &ctx) {
            Dispatch::Consume(p) => self.plan(&p),
            Dispatch::Pass(p) => {
                self.plan(&p);
                native(self);
            }
        }
    }
    fn run(&mut self, script: &str) -> String {
        for ch in script.chars() {
            match ch {
                '<' => self.key(KeyEvent::backspace(), |s| {
                    if s.c > 0 {
                        s.c -= 1;
                        s.s.remove(s.c);
                    }
                }),
                '{' => self.key(KeyEvent::left(), |s| {
                    if s.c > 0 {
                        s.c -= 1
                    }
                }),
                '}' => self.key(KeyEvent::right(), |s| {
                    if s.c < s.s.len() {
                        s.c += 1
                    }
                }),
                '\n' => self.key(KeyEvent::enter(), |s| {
                    s.s.insert(s.c, '\n');
                    s.c += 1;
                }),
                c => self.key(KeyEvent::char(c), move |s| {
                    s.s.insert(s.c, c);
                    s.c += 1;
                }),
            }
        }
        let mut out: String = self.s.iter().collect();
        out.insert(
            out.char_indices()
                .nth(self.c)
                .map(|x| x.0)
                .unwrap_or(out.len()),
            '|',
        );
        out
    }
}
fn main() {
    let cases: &[(&str, &str)] = &[
        // --- post-commit editing across punctuation / double separators ---
        ("ddi hoc. {{j", "đi học|. "),
        ("vieet. {{j", "việt|. "),
        ("tieng  {{j", "tiệng|  "),
        // --- editing an OLDER word keeps the newer ones editable ---
        ("vieet nam {{{{{j}}}}}{j", "việt nạm| "),
        // --- backspace re-opens a committed word ---
        // "viee" re-renders through the commit rules, and an open "viê" is
        // not a Vietnamese word, so the restore-raw rule keeps "vieej" (the
        // same text a fresh "vieej" + Space would show).
        ("vieet <<j", "việ|"),
        ("vieet <<tj", "việt|"),
        // Backspace onto the word end undoes its last keystroke; retyping the
        // same key re-renders the word ("hoc" + 'c' would be "hocc").
        ("hoc <<c", "hoc|"),
        // --- plain word-end edits ---
        ("vieet {j", "việt| "),
        ("vieet{j", "việt|"),
        ("vieet <j", "việt|"),
        // Two edits in a row (j then s: a second j would cancel the tone and
        // leave a literal 'j').
        ("vieet {js", "viết| "),
        ("vieet,{j", "việt|,"),
        // --- older words in a longer sentence ---
        ("chao ban {{{{{f", "chào| ban "),
        ("ban, toi {{{{{{f", "bàn|, toi "),
        ("moot hai ba {{{{{{{{j", "một| hai ba "),
        ("thuong {w", "thương| "),
        ("nguoi {{{w", "ngươi| "),
        ("Toi ddi. Hoc {j", "Toi đi. Học| "),
        ("ddi hoc {j", "đi học| "),
        // --- words that restore to their literal keystrokes ---
        ("law {{{f", "lằ| "),
        // --- carets outside the tracked geometry: pass through ---
        ("chao ban {{{{{{{{{x", "x|chao ban "),
        ("ab cd {{{{{{{{x", "x|ab cd "),
    ];
    let mut failures = 0;
    for (script, want) in cases {
        let mut sim = Sim::new();
        let got = sim.run(script);
        let ok = got == *want;
        if !ok {
            failures += 1;
        }
        println!(
            "{} {:<32} → {:<20} (want {})",
            if ok { "OK" } else { "XX" },
            format!("{script:?}"),
            format!("{got:?}"),
            want
        );
    }
    println!("{failures} failure(s)");
}
