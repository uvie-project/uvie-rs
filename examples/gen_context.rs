//! Sentence-context English table generator.
//!
//! Writes `src/tables/english_context.rs` (header + all tables + lookups +
//! unit tests) to the path given as `argv[1]`.
//!
//! Four tables back the English restore rules at word commit:
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
//! - `ENGLISH_AMBIGUOUS_WORDS` — restore candidates with their Vietnamese
//!   rarity score: the first 20,000 lines, lowercase ASCII letters, at least
//!   2 chars, whose engine render differs from the raw and contains a
//!   non-ASCII char, and that are NOT already in `ENGLISH_OVERRIDES` (the per-word
//!   dictionary path). The score is the number of whitespace tokens of
//!   `tests/data/vietnamese_22k.txt` equal to the lowercased render: "ít" has
//!   21, "tét" 2, "ỏ" 0 — a low score means the Vietnamese reading is
//!   essentially unused, so the English word wins at commit.
//!
//! - `ENGLISH_DOUBLED_KEY_WORDS` — English words containing a doubled Telex
//!   tone key pair (ss ff rr xx jj) whose render differs from the raw: the
//!   doubled key is the Telex cancel sequence, so "off" renders "of" and
//!   "diff" renders "dif". A real English word must show its literal
//!   keystrokes. Scanned over the whole corpus: "diff"/"lossless" sit past
//!   line 30k of `english_100k.txt`.
//!
//! - `VIETNAMESE_COMMON_SYLLABLES` — lowercased tokens of
//!   `tests/data/vietnamese_22k.txt` seen at least 6 times ("common
//!   syllables"). An ALL-CAPS acronym whose render is a rare syllable
//!   ("JIRA" → "JỈA") is English; one whose render is common ("VIỆT") is
//!   Vietnamese.
use std::collections::{HashMap, HashSet};
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

/// Telex tone keys: a doubled one is the cancel sequence ("off" → "of").
const TONE_KEYS: [char; 5] = ['s', 'f', 'r', 'x', 'j'];

/// A syllable seen at least this often in the Vietnamese word list counts as
/// common — an acronym whose render is common stays Vietnamese.
const COMMON_SYLLABLE_MIN: usize = 6;

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

/// True when the word contains a doubled Telex tone key ("off", "diff").
fn has_doubled_tone_key(word: &str) -> bool {
    let chars: Vec<char> = word.chars().collect();
    chars
        .windows(2)
        .any(|w| w[0] == w[1] && TONE_KEYS.contains(&w[0]))
}

fn main() {
    // Every whitespace token of the 22k word list, lowercased, with its
    // count: a Vietnamese token must never count as English context, and the
    // count is the Vietnamese rarity score of a render.
    let vi_data = include_str!("../tests/data/vietnamese_22k.txt");
    let mut vi_counts: HashMap<String, usize> = HashMap::new();
    for token in vi_data.lines().flat_map(|l| l.split_whitespace()) {
        *vi_counts.entry(token.to_lowercase()).or_default() += 1;
    }
    let overrides = current_overrides();
    let loanwords: HashSet<&str> = LOANWORDS.iter().copied().collect();

    let en_data = include_str!("../tests/data/english_100k.txt");
    let mut context: Vec<String> = Vec::new();
    let mut ambiguous: Vec<(String, u8)> = Vec::new();
    let mut doubled: Vec<String> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    for (line_no, line) in en_data.lines().enumerate() {
        let word = line.trim();
        if word.is_empty() || !seen.insert(word) {
            continue;
        }
        // Context words: ASCII lowercase, apostrophe allowed.
        let context_ok =
            line_no < 10_000 && word.chars().all(|c| c.is_ascii_lowercase() || c == '\'');
        // Restore candidates: ASCII lowercase letters only, at least 2 chars.
        let ambiguous_ok =
            line_no < 20_000 && word.len() >= 2 && word.chars().all(|c| c.is_ascii_lowercase());
        let doubled_ok = has_doubled_tone_key(word);
        if !context_ok && !ambiguous_ok && !doubled_ok {
            continue;
        }
        let rendered = render(word);
        if context_ok
            && rendered == word
            && (!vi_counts.contains_key(word) || FORCE_CONTEXT.contains(&word))
            && !loanwords.contains(word)
        {
            context.push(word.to_string());
        }
        if ambiguous_ok && rendered != word && !rendered.is_ascii() && !overrides.contains(word) {
            let score = vi_counts
                .get(&rendered.to_lowercase())
                .copied()
                .unwrap_or(0)
                .min(u8::MAX as usize) as u8;
            ambiguous.push((word.to_string(), score));
        }
        if doubled_ok && rendered != word {
            doubled.push(word.to_string());
        }
    }
    context.sort();
    context.dedup();
    ambiguous.sort();
    ambiguous.dedup();
    doubled.sort();
    doubled.dedup();
    let mut common: Vec<String> = vi_counts
        .iter()
        .filter(|&(_, &n)| n >= COMMON_SYLLABLE_MIN)
        .map(|(w, _)| w.clone())
        .collect();
    common.sort();

    println!("context words: {}", context.len());
    println!("ambiguous words: {}", ambiguous.len());
    println!("doubled-key words: {}", doubled.len());
    println!("common syllables: {}", common.len());
    let sample: Vec<&str> = ambiguous.iter().take(20).map(|(w, _)| w.as_str()).collect();
    println!("ambiguous sample: {sample:?}");
    let sample: Vec<&str> = doubled.iter().take(20).map(|s| s.as_str()).collect();
    println!("doubled sample: {sample:?}");

    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "src/tables/english_context.rs".to_string());
    let mut out = String::new();
    out.push_str(
        "//! English restore tables shared by the word-commit rules.\n\
         //!\n\
         //! Four sorted tables back the English restore rules at word commit\n\
         //! (`Diffable::commit_with_boundary_diff`), all gated by the host's\n\
         //! \"restore English\" setting:\n\
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
         //! - `ENGLISH_AMBIGUOUS_WORDS` — restore candidates with their\n\
         //!   Vietnamese rarity score: the first 20,000 lines, lowercase ASCII\n\
         //!   letters, length >= 2, whose engine render differs from the raw and\n\
         //!   contains a non-ASCII char, and that are NOT already in\n\
         //!   `ENGLISH_OVERRIDES` (the per-word dictionary path handles those).\n\
         //!   The score is the number of whitespace tokens of\n\
         //!   `tests/data/vietnamese_22k.txt` equal to the lowercased render\n\
         //!   (\"ít\" 21, \"tét\" 2, \"ỏ\" 0): a low score means the Vietnamese\n\
         //!   reading is essentially unused, so the English word wins.\n\
         //! - `ENGLISH_DOUBLED_KEY_WORDS` — English words containing a doubled\n\
         //!   Telex tone key pair (ss ff rr xx jj) whose render differs from\n\
         //!   the raw. The doubled key is the Telex cancel sequence, so\n\
         //!   \"off\" renders \"of\" and \"diff\" renders \"dif\"; a real English\n\
         //!   word must show its literal keystrokes.\n\
         //! - `VIETNAMESE_COMMON_SYLLABLES` — lowercased Vietnamese tokens seen\n\
         //!   at least 6 times. An ALL-CAPS acronym whose render is a rare\n\
         //!   syllable (\"JIRA\" → \"JỈA\") is English; one whose render is a\n\
         //!   common syllable (\"VIỆT\") is Vietnamese.\n\
         //!\n\
         //! All are sorted and looked up with a binary search (case-insensitive),\n\
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
         /// token (\"how\" → \"hơ\", \"best\" → \"bét\") with the render's Vietnamese\n\
         /// rarity score. {} entries.\n\
         #[rustfmt::skip]\n\
         static ENGLISH_AMBIGUOUS_WORDS: &[(&str, u8)] = &[\n",
        ambiguous.len()
    ));
    for (w, score) in &ambiguous {
        out.push_str(&format!(
            "    (\"{}\", {}),\n",
            w.replace('\\', "\\\\").replace('"', "\\\""),
            score
        ));
    }
    out.push_str("];\n\n");
    out.push_str(&format!(
        "/// English words containing a doubled Telex tone key whose render is\n\
         /// collapsed by the cancel sequence (\"off\" → \"of\"). {} entries.\n\
         #[rustfmt::skip]\n\
         static ENGLISH_DOUBLED_KEY_WORDS: &[&str] = &[\n",
        doubled.len()
    ));
    for w in &doubled {
        out.push_str(&format!(
            "    \"{}\",\n",
            w.replace('\\', "\\\\").replace('"', "\\\"")
        ));
    }
    out.push_str("];\n\n");
    out.push_str(&format!(
        "/// Lowercased Vietnamese tokens seen at least {} times in the word list —\n\
         /// the syllables an ALL-CAPS acronym must render to stay Vietnamese.\n\
         /// {} entries.\n\
         #[rustfmt::skip]\n\
         static VIETNAMESE_COMMON_SYLLABLES: &[&str] = &[\n",
        COMMON_SYLLABLE_MIN,
        common.len()
    ));
    for w in &common {
        out.push_str(&format!(
            "    \"{}\",\n",
            w.replace('\\', "\\\\").replace('"', "\\\"")
        ));
    }
    out.push_str("];\n\n");
    let mut loanword_table: Vec<&str> = LOANWORDS.to_vec();
    loanword_table.sort_unstable();
    loanword_table.dedup();
    out.push_str(&format!(
        "/// English loanwords Vietnamese speakers mix into Vietnamese sentences\n\
         /// (\"file lát\", \"ok rét\"): a sentence that contains one is still\n\
         /// Vietnamese, so an ambiguous word after it keeps its marks.\n\
         /// {} entries.\n\
         #[rustfmt::skip]\n\
         static VIETNAMESE_LOANWORDS: &[&str] = &[\n",
        loanword_table.len()
    ));
    for w in &loanword_table {
        out.push_str(&format!("    \"{}\",\n", w));
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
         /// Compare two `&str`s case-insensitively (Unicode lowercase, so\n\
         /// \"VIỆT\" matches the stored \"việt\").\n\
         #[inline]\n\
         fn compare_str_to_str(a: &str, b: &str) -> Ordering {\n\
         \x20   let mut bi = b.chars();\n\
         \x20   for ac in a.chars() {\n\
         \x20       let ac = ac.to_lowercase().next().unwrap_or(ac);\n\
         \x20       match bi.next() {\n\
         \x20           Some(bc) => match ac.cmp(&bc) {\n\
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
         fn find_chars(table: &[&str], word: &[char]) -> Option<usize> {\n\
         \x20   let mut lo = 0usize;\n\
         \x20   let mut hi = table.len();\n\
         \x20   while lo < hi {\n\
         \x20       let mid = (lo + hi) / 2;\n\
         \x20       match compare_chars_to_str(word, table[mid]) {\n\
         \x20           Ordering::Equal => return Some(mid),\n\
         \x20           Ordering::Less => hi = mid,\n\
         \x20           Ordering::Greater => lo = mid + 1,\n\
         \x20       }\n\
         \x20   }\n\
         \x20   None\n\
         }\n\n\
         #[inline]\n\
         fn contains_chars(table: &[&str], word: &[char]) -> bool {\n\
         \x20   find_chars(table, word).is_some()\n\
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
         \x20   english_ambiguous_score(word).is_some()\n\
         }\n\n\
         /// Vietnamese rarity score of `word`'s render: 0 when the render is not a\n\
         /// Vietnamese token at all, 21 for \"ít\", 2 for \"tét\". `None` when the\n\
         /// word is not an English restore candidate.\n\
         ///\n\
         /// Case-insensitive. Only called at word boundaries (space/commit).\n\
         #[inline]\n\
         pub fn english_ambiguous_score(word: &[char]) -> Option<u8> {\n\
         \x20   if word.len() < 2 {\n\
         \x20       return None;\n\
         \x20   }\n\
         \x20   let mut lo = 0usize;\n\
         \x20   let mut hi = ENGLISH_AMBIGUOUS_WORDS.len();\n\
         \x20   while lo < hi {\n\
         \x20       let mid = (lo + hi) / 2;\n\
         \x20       match compare_chars_to_str(word, ENGLISH_AMBIGUOUS_WORDS[mid].0) {\n\
         \x20           Ordering::Equal => return Some(ENGLISH_AMBIGUOUS_WORDS[mid].1),\n\
         \x20           Ordering::Less => hi = mid,\n\
         \x20           Ordering::Greater => lo = mid + 1,\n\
         \x20       }\n\
         \x20   }\n\
         \x20   None\n\
         }\n\n\
         /// Returns `true` when `word` (as a char slice of the raw keystrokes) is\n\
         /// an English word containing a doubled Telex tone key (\"off\", \"diff\").\n\
         ///\n\
         /// Case-insensitive. Only called at word boundaries (space/commit).\n\
         #[inline]\n\
         pub fn is_english_doubled_key_word(word: &[char]) -> bool {\n\
         \x20   !word.is_empty() && contains_chars(ENGLISH_DOUBLED_KEY_WORDS, word)\n\
         }\n\n\
         /// Returns `true` when `word` (a rendered syllable, any case) is a common\n\
         /// Vietnamese token — seen at least 6 times in the word list.\n\
         ///\n\
         /// Only called at word boundaries (space/commit).\n\
         #[inline]\n\
         pub fn is_common_vietnamese_syllable(word: &str) -> bool {\n\
         \x20   !word.is_empty() && contains_str(VIETNAMESE_COMMON_SYLLABLES, word)\n\
         }\n\n\
         /// Returns `true` when `word` (the previous committed word, as rendered)\n\
         /// is a curated English loanword Vietnamese speakers mix into Vietnamese\n\
         /// sentences: the sentence is still Vietnamese.\n\
         ///\n\
         /// Case-insensitive. Only called at word boundaries (space/commit).\n\
         #[inline]\n\
         pub fn is_vietnamese_loanword(word: &str) -> bool {\n\
         \x20   !word.is_empty() && contains_str(VIETNAMESE_LOANWORDS, word)\n\
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
         \x20   fn test_ambiguous_scores() {\n\
         \x20       // Rare Vietnamese readings: the English word wins.\n\
         \x20       assert_eq!(english_ambiguous_score(&['t', 'e', 's', 't']), Some(2));\n\
         \x20       assert_eq!(english_ambiguous_score(&['o', 'f']), Some(1));\n\
         \x20       assert_eq!(english_ambiguous_score(&['o', 'r']), Some(0));\n\
         \x20       // A common Vietnamese syllable scores high.\n\
         \x20       assert!(english_ambiguous_score(&['i', 't', 's']).unwrap() >= 6);\n\
         \x20       assert_eq!(english_ambiguous_score(&['a']), None);\n\
         \x20       assert_eq!(english_ambiguous_score(&['x', 'y', 'z', 'z']), None);\n\
         \x20   }\n\n\
         \x20   #[test]\n\
         \x20   fn test_doubled_key_lookup() {\n\
         \x20       for w in [\"off\", \"diff\", \"lossless\", \"affair\", \"affect\"] {\n\
         \x20           let chars: Vec<char> = w.chars().collect();\n\
         \x20           assert!(is_english_doubled_key_word(&chars), \"{w}\");\n\
         \x20       }\n\
         \x20       // Words the cancel sequence leaves alone have nothing to fix.\n\
         \x20       for w in [\"staff\", \"process\", \"class\"] {\n\
         \x20           let chars: Vec<char> = w.chars().collect();\n\
         \x20           assert!(!is_english_doubled_key_word(&chars), \"{w}\");\n\
         \x20       }\n\
         \x20       // \"masster\" is not an English word: the double-s cancel must stay.\n\
         \x20       assert!(!is_english_doubled_key_word(&['m', 'a', 's', 's', 't', 'e', 'r']));\n\
         \x20       assert!(!is_english_doubled_key_word(&['m', 'a', 's', 't', 'e', 'r']));\n\
         \x20       assert!(!is_english_doubled_key_word(&[]));\n\
         \x20   }\n\n\
         \x20   #[test]\n\
         \x20   fn test_common_syllables() {\n\
         \x20       assert!(is_common_vietnamese_syllable(\"việt\"));\n\
         \x20       assert!(is_common_vietnamese_syllable(\"VIỆT\"));\n\
         \x20       assert!(is_common_vietnamese_syllable(\"hà\"));\n\
         \x20       assert!(!is_common_vietnamese_syllable(\"jỉa\")); // JIRA's render\n\
         \x20       assert!(!is_common_vietnamese_syllable(\"\"));\n\
         \x20   }\n\n\
         \x20   #[test]\n\
         \x20   fn test_loanword_lookup() {\n\
         \x20       assert!(is_vietnamese_loanword(\"ok\"));\n\
         \x20       assert!(is_vietnamese_loanword(\"file\"));\n\
         \x20       assert!(is_vietnamese_loanword(\"OK\")); // case-insensitive\n\
         \x20       assert!(!is_vietnamese_loanword(\"hello\"));\n\
         \x20       assert!(!is_vietnamese_loanword(\"\"));\n\
         \x20   }\n\n\
         \x20   #[test]\n\
         \x20   fn tables_are_sorted_and_lowercase() {\n\
         \x20       for w in ENGLISH_CONTEXT_WORDS\n\
         \x20           .iter()\n\
         \x20           .chain(ENGLISH_DOUBLED_KEY_WORDS)\n\
         \x20           .chain(VIETNAMESE_COMMON_SYLLABLES)\n\
         \x20           .chain(VIETNAMESE_LOANWORDS)\n\
         \x20       {\n\
         \x20           assert!(w.chars().all(|c| !c.is_ascii_uppercase()));\n\
         \x20       }\n\
         \x20       for (w, _) in ENGLISH_AMBIGUOUS_WORDS {\n\
         \x20           assert!(w.chars().all(|c| c.is_ascii_lowercase()));\n\
         \x20       }\n\
         \x20       assert!(ENGLISH_CONTEXT_WORDS.windows(2).all(|w| w[0] < w[1]));\n\
         \x20       assert!(ENGLISH_DOUBLED_KEY_WORDS.windows(2).all(|w| w[0] < w[1]));\n\
         \x20       assert!(VIETNAMESE_COMMON_SYLLABLES.windows(2).all(|w| w[0] < w[1]));\n\
         \x20       assert!(VIETNAMESE_LOANWORDS.windows(2).all(|w| w[0] < w[1]));\n\
         \x20       assert!(ENGLISH_AMBIGUOUS_WORDS\n\
         \x20           .windows(2)\n\
         \x20           .all(|w| w[0].0 < w[1].0));\n\
         \x20   }\n\
         }\n",
    );

    use std::io::Write;
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(out.as_bytes()).unwrap();
    println!("wrote {} bytes to {}", out.len(), path);
}
