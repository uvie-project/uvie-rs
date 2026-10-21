//! Randomized invariant tests for the diff API.
//!
//! The host applies every diff `(backspaces, suffix)` to the screen, so the
//! engine's own view of the screen (`current_output_diff`) must always equal
//! the text those instructions produced — a mismatch is a ghost character or
//! a lost letter. The sequences mix keystrokes, backspaces, word boundaries
//! and the host's Escape restore (erase the render, post the raw keystrokes)
//! across Telex, VNI, Simple Telex and uppercase input, with an alphabet
//! biased to the dictionary/cancel machinery.
use uvie::InputMethod;
use uvie::UltraFastViEngine;
use uvie::diff::Diffable;

fn apply(screen: &mut String, bs: usize, suffix: &str) {
    let sc: Vec<char> = screen.chars().collect();
    *screen = sc[..sc.len().saturating_sub(bs)].iter().collect::<String>();
    screen.push_str(suffix);
}

/// Small deterministic PRNG so failures reproduce from the seed.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[(self.next() as usize) % items.len()]
    }
}

/// Types a random sequence for one seed, asserting the screen invariant
/// after every action.
fn run(alphabet: &[char], method: InputMethod, seed: u64, steps: usize) {
    let mut rng = Lcg(seed);
    let mut engine = UltraFastViEngine::new();
    engine.set_input_method(method);
    let mut screen = String::new();
    // Text on screen that is left of the current word (committed words).
    let mut prefix = String::new();
    let mut typed: Vec<char> = Vec::new();
    for _ in 0..steps {
        let roll = rng.next() % 100;
        if roll < 10 {
            let (bs, suffix) = engine.backspace_diff();
            let suffix = suffix.to_string();
            apply(&mut screen, bs, &suffix);
            typed.pop();
        } else if roll < 15 {
            let (bs, suffix) = engine.feed_diff(' ');
            let suffix = suffix.to_string();
            apply(&mut screen, bs, &suffix);
            prefix = screen.clone();
            typed.clear();
            assert!(
                engine.word_raw_string().is_empty(),
                "word_raw survived a boundary: {:?}",
                engine.word_raw_string()
            );
        } else if roll < 18 {
            if engine.is_composing_diff() {
                let out = engine.current_output_diff().to_string();
                let raw = engine.word_raw_string().to_string();
                if !raw.is_empty() && raw != out {
                    apply(&mut screen, out.chars().count(), &raw);
                    assert_eq!(
                        screen,
                        format!("{}{}", prefix, raw),
                        "escape restore mismatch (seed {})",
                        seed
                    );
                }
                engine.reset_diff();
                // The restored literal text stays on screen as committed text.
                prefix = screen.clone();
                typed.clear();
                assert!(!engine.is_composing_diff());
            }
        } else {
            let ch = rng.pick(alphabet);
            let (bs, suffix) = engine.feed_diff(ch);
            let suffix = suffix.to_string();
            apply(&mut screen, bs, &suffix);
            typed.push(ch);
        }
        let engine_view = engine.current_output_diff().to_string();
        let expected = format!("{}{}", prefix, engine_view);
        assert_eq!(
            expected, screen,
            "screen drift after {:?} (seed {})",
            typed, seed
        );
        let raw = engine.word_raw_string().to_string();
        assert!(
            raw.len() <= typed.len(),
            "word_raw longer than typed: {:?} vs {:?} (seed {})",
            raw,
            typed,
            seed
        );
    }
}

#[test]
fn invariants_telex() {
    let alphabet: Vec<char> = "abcdeghiklmnopqrstuvxywzsfrj".chars().collect();
    for seed in 0..600u64 {
        run(&alphabet, InputMethod::Telex, seed, 16);
    }
}

#[test]
fn invariants_vni() {
    let alphabet: Vec<char> = "abcdeghiklmnopqrstuvxywz0123456789".chars().collect();
    for seed in 0..600u64 {
        run(&alphabet, InputMethod::Vni, seed, 16);
    }
}

#[test]
fn invariants_simple_telex() {
    let alphabet: Vec<char> = "abcdeghiklmnopqrstuvxywz".chars().collect();
    for seed in 0..300u64 {
        run(&alphabet, InputMethod::SimpleTelex, seed, 16);
    }
}

#[test]
fn invariants_uppercase() {
    let alphabet: Vec<char> = "ABCDeghiklmnopqrstuvxyzsf".chars().collect();
    for seed in 0..300u64 {
        run(&alphabet, InputMethod::Telex, seed, 16);
    }
}

#[test]
fn invariants_cancel_heavy_long_words() {
    // Alphabet biased to the cancel/dictionary machinery: doubled tone keys,
    // dictionary prefixes, long words (up to the 24-char buffer).
    let alphabet: Vec<char> = "maspflosernivxt".chars().collect();
    for seed in 0..800u64 {
        run(&alphabet, InputMethod::Telex, seed, 26);
    }
}
