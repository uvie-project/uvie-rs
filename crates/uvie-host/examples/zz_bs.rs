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
    fn key(&mut self, k: KeyEvent, native: impl FnOnce(&mut Sim)) -> &'static str {
        let ctx = Context {
            language: None,
            app_excluded: false,
            non_latin_layout: false,
        };
        match self.d.handle(k, &ctx) {
            Dispatch::Consume(p) => {
                self.plan(&p);
                "C"
            }
            Dispatch::Pass(p) => {
                self.plan(&p);
                native(self);
                "P"
            }
        }
    }
    fn scr(&self) -> String {
        let mut o: String = self.s.iter().collect();
        let i = o.char_indices().nth(self.c).map(|x| x.0).unwrap_or(o.len());
        o.insert(i, '|');
        o
    }
}
fn main() {
    for script in std::env::args().skip(1) {
        let mut sim = Sim {
            d: Dispatcher::new(EngineSession::with_options(
                InputMethod::Telex,
                EngineOptions {
                    english_override: true,
                    ..Default::default()
                },
            )),
            s: vec![],
            c: 0,
        };
        let mut trace = vec![];
        for ch in script.chars() {
            let r = match ch {
                '<' => sim.key(KeyEvent::backspace(), |s| {
                    if s.c > 0 {
                        s.c -= 1;
                        s.s.remove(s.c);
                    }
                }),
                '{' => sim.key(KeyEvent::left(), |s| {
                    if s.c > 0 {
                        s.c -= 1
                    }
                }),
                c => sim.key(KeyEvent::char(c), move |s| {
                    s.s.insert(s.c, c);
                    s.c += 1
                }),
            };
            if ch == '<' {
                trace.push(format!("{}{}", r, sim.scr()));
            }
        }
        println!("{script:?} → {}\n   bs: {}", sim.scr(), trace.join("  "));
    }
}
