//! Vietnamese round-trip harness: converts Vietnamese text to Telex
//! keystrokes, types it through the dispatcher, and reports lines whose
//! screen text differs from the source text.
use std::collections::BTreeMap;
use uvie::InputMethod;
use uvie_host::dispatcher::{Context, Dispatch, Dispatcher, InputAction};
use uvie_host::engine_session::{EngineOptions, EngineSession};
use uvie_host::keys::KeyEvent;

/// (form without tone, base letter, modifier key) for every Vietnamese vowel
/// the converter understands.
const FORMS: [(char, char, Option<char>); 12] = [
    ('a', 'a', None),
    ('ă', 'a', Some('w')),
    ('â', 'a', Some('a')),
    ('e', 'e', None),
    ('ê', 'e', Some('e')),
    ('i', 'i', None),
    ('o', 'o', None),
    ('ô', 'o', Some('o')),
    ('ơ', 'o', Some('w')),
    ('u', 'u', None),
    ('ư', 'u', Some('w')),
    ('y', 'y', None),
];

const TONE_FORMS: [[char; 6]; 12] = [
    ['a', 'á', 'à', 'ả', 'ã', 'ạ'],
    ['ă', 'ắ', 'ằ', 'ẳ', 'ẵ', 'ặ'],
    ['â', 'ấ', 'ầ', 'ẩ', 'ẫ', 'ậ'],
    ['e', 'é', 'è', 'ẻ', 'ẽ', 'ẹ'],
    ['ê', 'ế', 'ề', 'ể', 'ễ', 'ệ'],
    ['i', 'í', 'ì', 'ỉ', 'ĩ', 'ị'],
    ['o', 'ó', 'ò', 'ỏ', 'õ', 'ọ'],
    ['ô', 'ố', 'ồ', 'ổ', 'ỗ', 'ộ'],
    ['ơ', 'ớ', 'ờ', 'ở', 'ỡ', 'ợ'],
    ['u', 'ú', 'ù', 'ủ', 'ũ', 'ụ'],
    ['ư', 'ứ', 'ừ', 'ử', 'ữ', 'ự'],
    ['y', 'ý', 'ỳ', 'ỷ', 'ỹ', 'ỵ'],
];

const TONE_KEYS: [char; 6] = ['\0', 's', 'f', 'r', 'x', 'j'];

/// Telex keystrokes for one Vietnamese word: modifier keys right after the
/// base letter, tone keys at the end of the word.
fn telex_word(word: &str) -> String {
    let mut out = String::new();
    let mut tones = String::new();
    for c in word.chars() {
        let lower = c.to_lowercase().next().unwrap_or(c);
        let upper = c.is_uppercase();
        if lower == 'đ' {
            out.push(if upper { 'D' } else { 'd' });
            out.push(if upper { 'D' } else { 'd' });
            continue;
        }
        let mut mapped = false;
        for (i, (plain, base, modifier)) in FORMS.iter().enumerate() {
            let Some(tone) = TONE_FORMS[i].iter().position(|&f| f == lower) else {
                continue;
            };
            let base = if upper {
                base.to_ascii_uppercase()
            } else {
                *base
            };
            out.push(base);
            if let Some(m) = modifier {
                if *m == *plain {
                    out.push(base);
                } else {
                    out.push(*m);
                }
            }
            if tone > 0 {
                tones.push(TONE_KEYS[tone]);
            }
            mapped = true;
            break;
        }
        if !mapped {
            out.push(c);
        }
    }
    out.push_str(&tones);
    out
}

fn telex_line(line: &str) -> String {
    let mut out = String::new();
    let mut word = String::new();
    for c in line.chars() {
        if c.is_alphabetic() {
            word.push(c);
        } else {
            if !word.is_empty() {
                out.push_str(&telex_word(&word));
                word.clear();
            }
            out.push(c);
        }
    }
    if !word.is_empty() {
        out.push_str(&telex_word(&word));
    }
    out
}

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
    let mut total = 0usize;
    let mut bad = 0usize;
    let mut shown = 0usize;
    let mut classes: BTreeMap<String, usize> = BTreeMap::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        total += 1;
        let typed = telex_line(line);
        let out = run(&format!("{typed} "));
        let out = out.trim_end_matches(' ');
        if out != line {
            bad += 1;
            let a: Vec<&str> = line.split(' ').collect();
            let b: Vec<&str> = out.split(' ').collect();
            let mut noted = false;
            for (x, y) in a.iter().zip(b.iter()) {
                if x != y {
                    *classes.entry(format!("{x}→{y}")).or_default() += 1;
                    noted = true;
                }
            }
            if !noted {
                *classes.entry("<length>".to_string()).or_default() += 1;
            }
            if shown < 20 {
                shown += 1;
                println!("want: {line}");
                println!("got : {out}");
                println!("keys: {typed}");
            }
        }
    }
    println!("lines {total} mismatched {bad}");
    let mut v: Vec<_> = classes.into_iter().collect();
    v.sort_by_key(|a| std::cmp::Reverse(a.1));
    for (k, n) in v.iter().take(40) {
        print!("{k}({n})  ");
    }
    println!();
}
