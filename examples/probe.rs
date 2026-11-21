//! Probe: cargo run --release --example probe -- [telex|vni] "keys" ...
//! Prints the screen after every keystroke.
use uvie::diff::Diffable;
use uvie::{InputMethod, UltraFastViEngine};
fn main() {
    let mut args = std::env::args().skip(1);
    let m = args.next().unwrap();
    for text in args {
        let mut e = UltraFastViEngine::new();
        e.set_input_method(if m == "vni" {
            InputMethod::Vni
        } else {
            InputMethod::Telex
        });
        let mut screen: Vec<char> = Vec::new();
        let mut trace = String::new();
        for ch in text.chars() {
            let (bs, s) = if ch == '\u{8}' || ch == '<' {
                e.backspace_diff()
            } else {
                e.feed_diff(ch)
            };
            let s = s.to_string();
            let n = screen.len().saturating_sub(bs);
            screen.truncate(n);
            screen.extend(s.chars());
            trace.push_str(&format!("{}|", screen.iter().collect::<String>()));
        }
        println!(
            "{text:?} => {:?}\n    {trace}",
            screen.iter().collect::<String>()
        );
    }
}
