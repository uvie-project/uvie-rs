use std::collections::BTreeMap;
use uvie::InputMethod;
use uvie_host::dispatcher::{Context, Dispatch, Dispatcher, InputAction};
use uvie_host::engine_session::{EngineOptions, EngineSession};
use uvie_host::keys::KeyEvent;
fn run(text: &str) -> String {
    let mut d = Dispatcher::new(EngineSession::with_options(
        InputMethod::Telex,
        EngineOptions {
            english_override: true,
            ..Default::default()
        },
    ));
    let mut s: Vec<char> = vec![];
    let mut c = 0usize;
    let ctx = Context {
        language: None,
        app_excluded: false,
        non_latin_layout: false,
    };
    for ch in text.chars() {
        let k = if ch == '\n' {
            KeyEvent::enter()
        } else {
            KeyEvent::char(ch)
        };
        let (plan, pass) = match d.handle(k, &ctx) {
            Dispatch::Consume(p) => (p, false),
            Dispatch::Pass(p) => (p, true),
        };
        for a in plan {
            match a {
                InputAction::Backspace(n) => {
                    for _ in 0..n {
                        if c > 0 {
                            c -= 1;
                            s.remove(c);
                        }
                    }
                }
                InputAction::ForwardDelete(n) => {
                    for _ in 0..n {
                        if c < s.len() {
                            s.remove(c);
                        }
                    }
                }
                InputAction::Text(t) => {
                    for x in t.chars() {
                        s.insert(c, x);
                        c += 1;
                    }
                }
                #[allow(unreachable_patterns)]
                _ => {}
            }
        }
        if pass {
            s.insert(c, ch);
            c += 1;
        }
    }
    s.into_iter().collect()
}
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let text = std::fs::read_to_string(path).unwrap();
    let mut bad: BTreeMap<String, (String, usize)> = BTreeMap::new();
    let mut total = 0;
    let mut nbad = 0;
    for line in text.lines() {
        let line: String = line
            .chars()
            .filter(|c| c.is_ascii() && !c.is_ascii_control())
            .collect();
        if line.trim().is_empty() {
            continue;
        }
        let out = run(&format!("{line} "));
        let a: Vec<&str> = line.split(' ').collect();
        let b: Vec<&str> = out.trim_end_matches(' ').split(' ').collect();
        if a.len() != b.len() {
            continue;
        }
        for (x, y) in a.iter().zip(b.iter()) {
            total += 1;
            if x != y {
                nbad += 1;
                let e = bad.entry(x.to_string()).or_insert((y.to_string(), 0));
                e.1 += 1;
            }
        }
    }
    let mut v: Vec<_> = bad.into_iter().collect();
    v.sort_by(|a, b| b.1.1.cmp(&a.1.1));
    println!("words {total} wrong {nbad} distinct {}", v.len());
    for (w, (r, n)) in v.iter().take(150) {
        print!("{w}→{r}({n})  ");
    }
    println!();
}
