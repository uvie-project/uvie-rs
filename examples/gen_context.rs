//! Sentence-context English table generator.
//!
//! Writes `src/tables/english_context.rs` (header + both tables + lookups +
//! unit tests) to the path given as `argv[1]`.
//!
//! Two tables back the sentence-context English restore rule at word commit:
//!
//! - `ENGLISH_CONTEXT_WORDS` — words that, as the PREVIOUS committed word,
//!   signal English context: from the first 10,000 lines of
//!   `tests/data/english_100k.txt`, lowercase ASCII letters (apostrophe
//!   allowed), that the engine passes through UNCHANGED (feed_diff render ==
//!   raw, English override off), that are NOT a Vietnamese word (every
//!   whitespace token of `tests/data/vietnamese_22k.txt`, lowercased), and
//!   that are NOT in the curated loanword list commonly mixed into
//!   Vietnamese sentences.
//!
//! - `ENGLISH_AMBIGUOUS_WORDS` — restore candidates: same first 10,000
//!   lines, lowercase ASCII letters, length >= 2, whose engine render differs
//!   from the raw and contains a non-ASCII char, and that are NOT already in
//!   `ENGLISH_OVERRIDES` (the per-word dictionary path).
use std::collections::HashSet;
use uvie::UltraFastViEngine;
use uvie::diff::Diffable;

/// Rare Vietnamese tokens that are far more frequent as English sentence
/// words ("the best way", "Hi, how are you"): kept as context words even
/// though they appear in the Vietnamese list.
const FORCE_CONTEXT: &[&str] = &["the", "hi"];

/// Curated loanwords that Vietnamese speakers mix into Vietnamese sentences.
/// They must not switch the sentence context to English ("file lát" stays
/// Vietnamese).
const LOANWORDS: &[&str] = &[
    "ok", "okay", "oke", "file", "email", "mail", "app", "web", "link", "page", "post", "like",
    "share", "game", "team", "check", "meeting", "deadline", "sale", "online", "offline", "video",
    "clip", "face", "chat", "code", "bug", "fix", "demo", "data", "server", "laptop", "phone",
    "iphone", "menu", "size", "shop", "ship", "order", "update", "test", "sorry", "thanks", "bye",
    "cafe", "coffee", "wifi", "pass", "level", "style",
];

// Override is disabled on the generator engine so we see the raw transform
// the tables are meant to fix (feed_diff applies the override per-keystroke).
fn type_word(engine: &mut UltraFastViEngine, input: &str) -> String {
    let mut screen = String::new();
    for c in input.chars() {
        let (bs, suffix) = engine.feed_diff(c);
        for _ in 0..bs {
            screen.pop();
        }
        screen.push_str(suffix);
    }
    screen
}

/// Words shipped in `src/tables/english.rs` — the per-word dictionary path.
fn current_overrides() -> HashSet<String> {
    include_str!("../src/tables/english.rs")
        .lines()
        .filter(|l| l.starts_with("    \""))
        .filter_map(|l| {
            l.trim()
                .trim_end_matches(',')
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
        })
        .map(|s| s.to_string())
        .collect()
}

fn render(word: &str) -> String {
    let mut e = UltraFastViEngine::new();
    e.set_english_override(false);
    type_word(&mut e, word)
}

fn main() {
    // Every whitespace token of the 22k word list, lowercased: a Vietnamese
    // token must never count as English context.
    let vi_data = include_str!("../tests/data/vietnamese_22k.txt");
    let vi_words: HashSet<String> = vi_data
        .lines()
        .flat_map(|l| l.split_whitespace())
        .map(|t| t.chars().map(|c| c.to_ascii_lowercase()).collect())
        .collect();
    let overrides = current_overrides();
    let loanwords: HashSet<&str> = LOANWORDS.iter().copied().collect();

    let en_data = include_str!("../tests/data/english_100k.txt");
    let mut context: Vec<String> = Vec::new();
    let mut ambiguous: Vec<String> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for line in en_data.lines().take(10_000) {
        let word = line.trim();
        if word.is_empty() || !seen.insert(word) {
            continue;
        }
        // Context words: ASCII lowercase, apostrophe allowed.
        let context_ok = word.chars().all(|c| c.is_ascii_lowercase() || c == '\'');
        // Restore candidates: ASCII lowercase letters only, at least 2 chars.
        let ambiguous_ok = word.len() >= 2 && word.chars().all(|c| c.is_ascii_lowercase());
        if !context_ok && !ambiguous_ok {
            continue;
        }
        let rendered = render(word);
        if context_ok
            && rendered == word
            && (!vi_words.contains(word) || FORCE_CONTEXT.contains(&word))
            && !loanwords.contains(word)
        {
            context.push(word.to_string());
        }
        if ambiguous_ok && rendered != word && !rendered.is_ascii() && !overrides.contains(word) {
            ambiguous.push(word.to_string());
        }
    }
    context.sort();
    context.dedup();
    ambiguous.sort();
    ambiguous.dedup();

    println!("context words: {}", context.len());
    println!("ambiguous words: {}", ambiguous.len());
    let sample: Vec<&str> = ambiguous.iter().take(30).map(|s| s.as_str()).collect();
    println!("ambiguous sample: {sample:?}");

    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "src/tables/english_context.rs".to_string());
    let mut out = String::new();
    out.push_str(
        "//! Sentence-context English preference tables.\n\
         //!\n\
         //! Two sorted tables back the sentence-context English restore rule at\n\
         //! word commit (`Diffable::commit_diff`): when the PREVIOUS committed\n\
         //! word is clearly English, a common English word that the pipeline\n\
         //! marked as Vietnamese is restored to its raw keystrokes\n\
         //! (\"Hello, how are you?\" must not render \"Hello, hơ are you?\").\n\
         //!\n\
         //! - `ENGLISH_CONTEXT_WORDS` — words that, as the previous committed\n\
         //!   word, signal English context. From the first 10,000 lines of\n\
         //!   `tests/data/english_100k.txt`: lowercase ASCII letters (apostrophe\n\
         //!   allowed) that the engine passes through UNCHANGED (feed_diff\n\
         //!   render == raw, English override off), excluding every whitespace\n\
         //!   token of `tests/data/vietnamese_22k.txt` (lowercased — a\n\
         //!   Vietnamese word must never flip the context) and the curated\n\
         //!   loanword list commonly mixed into Vietnamese sentences (ok, file,\n\
         //!   email, app, deadline, ...).\n\
         //! - `ENGLISH_AMBIGUOUS_WORDS` — restore candidates: same first 10,000\n\
         //!   lines, lowercase ASCII letters, length >= 2, whose engine render\n\
         //!   differs from the raw and contains a non-ASCII char, and that are\n\
         //!   NOT already in `ENGLISH_OVERRIDES` (the per-word dictionary path\n\
         //!   handles those).\n\
         //!\n\
         //! Both are sorted and looked up with a binary search (case-insensitive),\n\
         //! so the cost is paid only at word commit, never per keystroke.\n\
         //!\n\
         //! Regenerate with:\n\
         //! `cargo run --release --example gen_context -- src/tables/english_context.rs`\n\
         \n\
         use core::cmp::Ordering;\n\n",
    );
    out.push_str(&format!(
        "/// Words that, as the previous committed word, mark the sentence context\n\
         /// as English. {} entries.\n\
         #[rustfmt::skip]\n\
         static ENGLISH_CONTEXT_WORDS: &[&str] = &[\n",
        context.len()
    ));
    for w in &context {
        out.push_str(&format!(
            "    \"{}\",\n",
            w.replace('\\', "\\\\").replace('"', "\\\"")
        ));
    }
    out.push_str("];\n\n");
    out.push_str(&format!(
        "/// Common English words whose engine render is a marked Vietnamese-looking\n\
         /// token (\"how\" → \"hơ\", \"best\" → \"bét\") — restore candidates when the\n\
         /// previous word is English context. {} entries.\n\
         #[rustfmt::skip]\n\
         static ENGLISH_AMBIGUOUS_WORDS: &[&str] = &[\n",
        ambiguous.len()
    ));
    for w in &ambiguous {
        out.push_str(&format!(
            "    \"{}\",\n",
            w.replace('\\', "\\\\").replace('"', "\\\"")
        ));
    }
    out.push_str("];\n\n");
    out.push_str(
        "/// Compare a `[char]` slice against a `&str`, case-insensitively.\n\
         /// Returns `Equal` if they match (ignoring case), `Less`/`Greater` otherwise.\n\
         #[inline]\n\
         fn compare_chars_to_str(chars: &[char], s: &str) -> Ordering {\n\
         \x20   let mut si = s.chars();\n\
         \x20   for &c in chars {\n\
         \x20       match si.next() {\n\
         \x20           Some(sc) => match c.to_ascii_lowercase().cmp(&sc.to_ascii_lowercase()) {\n\
         \x20               Ordering::Equal => continue,\n\
         \x20               ord => return ord,\n\
         \x20           },\n\
         \x20           None => return Ordering::Greater,\n\
         \x20       }\n\
         \x20   }\n\
         \x20   if si.next().is_some() {\n\
         \x20       Ordering::Less\n\
         \x20   } else {\n\
         \x20       Ordering::Equal\n\
         \x20   }\n\
         }\n\n\
         /// Compare two `&str`s case-insensitively (ASCII case folding).\n\
         #[inline]\n\
         fn compare_str_to_str(a: &str, b: &str) -> Ordering {\n\
         \x20   let mut bi = b.chars();\n\
         \x20   for ac in a.chars() {\n\
         \x20       match bi.next() {\n\
         \x20           Some(bc) => match ac.to_ascii_lowercase().cmp(&bc.to_ascii_lowercase()) {\n\
         \x20               Ordering::Equal => continue,\n\
         \x20               ord => return ord,\n\
         \x20           },\n\
         \x20           None => return Ordering::Greater,\n\
         \x20       }\n\
         \x20   }\n\
         \x20   if bi.next().is_some() {\n\
         \x20       Ordering::Less\n\
         \x20   } else {\n\
         \x20       Ordering::Equal\n\
         \x20   }\n\
         }\n\n\
         #[inline]\n\
         fn contains_chars(table: &[&str], word: &[char]) -> bool {\n\
         \x20   let mut lo = 0usize;\n\
         \x20   let mut hi = table.len();\n\
         \x20   while lo < hi {\n\
         \x20       let mid = (lo + hi) / 2;\n\
         \x20       match compare_chars_to_str(word, table[mid]) {\n\
         \x20           Ordering::Equal => return true,\n\
         \x20           Ordering::Less => hi = mid,\n\
         \x20           Ordering::Greater => lo = mid + 1,\n\
         \x20       }\n\
         \x20   }\n\
         \x20   false\n\
         }\n\n\
         #[inline]\n\
         fn contains_str(table: &[&str], word: &str) -> bool {\n\
         \x20   let mut lo = 0usize;\n\
         \x20   let mut hi = table.len();\n\
         \x20   while lo < hi {\n\
         \x20       let mid = (lo + hi) / 2;\n\
         \x20       match compare_str_to_str(word, table[mid]) {\n\
         \x20           Ordering::Equal => return true,\n\
         \x20           Ordering::Less => hi = mid,\n\
         \x20           Ordering::Greater => lo = mid + 1,\n\
         \x20       }\n\
         \x20   }\n\
         \x20   false\n\
         }\n\n\
         /// Returns `true` when `word` (the previous committed word, as rendered\n\
         /// on screen) marks the sentence context as English.\n\
         ///\n\
         /// Case-insensitive. Uses binary search on a sorted static array.\n\
         /// Only called at word boundaries (space/commit), never per-keystroke.\n\
         #[inline]\n\
         pub fn is_english_context_word(word: &str) -> bool {\n\
         \x20   !word.is_empty() && contains_str(ENGLISH_CONTEXT_WORDS, word)\n\
         }\n\n\
         /// Returns `true` when `word` (as a char slice of the raw keystrokes)\n\
         /// is a common English word the pipeline may have marked as Vietnamese.\n\
         ///\n\
         /// Case-insensitive. Uses binary search on a sorted static array.\n\
         /// Only called at word boundaries (space/commit), never per-keystroke.\n\
         #[inline]\n\
         pub fn is_english_ambiguous_word(word: &[char]) -> bool {\n\
         \x20   word.len() >= 2 && contains_chars(ENGLISH_AMBIGUOUS_WORDS, word)\n\
         }\n\n",
    );
    out.push_str(
        "#[cfg(test)]\n\
         mod tests {\n\
         \x20   use super::*;\n\n\
         \x20   #[test]\n\
         \x20   fn test_context_lookup() {\n\
         \x20       assert!(is_english_context_word(\"hello\"));\n\
         \x20       assert!(is_english_context_word(\"YOU\")); // case-insensitive\n\
         \x20       assert!(is_english_context_word(\"are\"));\n\
         \x20       assert!(is_english_context_word(\"it\"));\n\
         \x20       assert!(is_english_context_word(\"hi\")); // forced context word\n\
         \x20       assert!(!is_english_context_word(\"than\")); // a Vietnamese token\n\
         \x20       assert!(!is_english_context_word(\"file\")); // curated loanword\n\
         \x20       assert!(!is_english_context_word(\"\"));\n\
         \x20   }\n\n\
         \x20   #[test]\n\
         \x20   fn test_ambiguous_lookup() {\n\
         \x20       assert!(is_english_ambiguous_word(&['h', 'o', 'w']));\n\
         \x20       assert!(is_english_ambiguous_word(&['B', 'E', 'S', 'T'])); // case-insensitive\n\
         \x20       assert!(is_english_ambiguous_word(&['a', 's']));\n\
         \x20       assert!(!is_english_ambiguous_word(&['a'])); // too short\n\
         \x20       assert!(!is_english_ambiguous_word(&['x', 'y', 'z', 'z']));\n\
         \x20       assert!(!is_english_ambiguous_word(&[]));\n\
         \x20   }\n\n\
         \x20   #[test]\n\
         \x20   fn tables_are_sorted_and_lowercase() {\n\
         \x20       for w in ENGLISH_CONTEXT_WORDS.iter().chain(ENGLISH_AMBIGUOUS_WORDS) {\n\
         \x20           assert!(w.chars().all(|c| c.is_ascii_lowercase() || c == '\\''));\n\
         \x20       }\n\
         \x20       assert!(ENGLISH_CONTEXT_WORDS.windows(2).all(|w| w[0] < w[1]));\n\
         \x20       assert!(ENGLISH_AMBIGUOUS_WORDS.windows(2).all(|w| w[0] < w[1]));\n\
         \x20   }\n\
         }\n",
    );

    use std::io::Write;
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(out.as_bytes()).unwrap();
    println!("wrote {} bytes to {}", out.len(), path);
}
