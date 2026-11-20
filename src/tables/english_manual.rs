//! Hand-curated English tables for the sentence-context rules.
//!
//! The generated tables in `english_context.rs` are inventories (thousands of
//! words); these are the small, deliberate lists the commit rules judge a
//! sentence by. Keep them sorted: the lookups are binary searches.

/// English function words that, as the word before an ambiguous Vietnamese
/// reading, prove the sentence is English ("the car", "my car is red").
///
/// Only these may restore a word whose render is a TOP Vietnamese syllable:
/// the reading is a word Vietnamese speakers type constantly, so it takes a
/// genuine English function word - not just any dictionary word - to flip it.
static ENGLISH_FUNCTION_WORDS: &[&str] = &[
    "a", "and", "any", "are", "as", "at", "be", "been", "but", "by", "each", "every", "for",
    "from", "he", "her", "his", "i", "in", "is", "it", "its", "my", "not", "of", "on", "or", "our",
    "she", "some", "that", "the", "their", "these", "they", "this", "those", "too", "very", "was",
    "we", "were", "with", "you", "your",
];

/// Lowercase tech words that are always English, whatever the previous word
/// is: they render as a rare Vietnamese syllable ("test" → "tét", "cost" →
/// "cót"), so a Vietnamese neighbour must not keep the reading.
///
/// The one exception is the Vietnamese dish "bánh tét": right after "bánh" the
/// render is what the user meant.
static ALWAYS_ENGLISH_TECH_WORDS: &[&str] = &[
    "boost", "cost", "costs", "host", "hosts", "list", "lists", "test", "tests",
];

#[inline]
fn contains_str(table: &[&str], word: &str) -> bool {
    let mut lo = 0usize;
    let mut hi = table.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        match compare_lowercase(word, table[mid]) {
            core::cmp::Ordering::Equal => return true,
            core::cmp::Ordering::Less => hi = mid,
            core::cmp::Ordering::Greater => lo = mid + 1,
        }
    }
    false
}

#[inline]
fn contains_chars(table: &[&str], word: &[char]) -> bool {
    let mut lo = 0usize;
    let mut hi = table.len();
    while lo < hi {
        let mid = (lo + hi) / 2;
        match compare_chars_to_str(word, table[mid]) {
            core::cmp::Ordering::Equal => return true,
            core::cmp::Ordering::Less => hi = mid,
            core::cmp::Ordering::Greater => lo = mid + 1,
        }
    }
    false
}

/// Case-insensitive `&str` comparison against a lowercase table entry.
#[inline]
fn compare_lowercase(word: &str, entry: &str) -> core::cmp::Ordering {
    let mut a = word.chars().flat_map(|c| c.to_lowercase());
    let mut b = entry.chars();
    loop {
        match (a.next(), b.next()) {
            (Some(ac), Some(bc)) => match ac.cmp(&bc) {
                core::cmp::Ordering::Equal => continue,
                ord => return ord,
            },
            (Some(_), None) => return core::cmp::Ordering::Greater,
            (None, Some(_)) => return core::cmp::Ordering::Less,
            (None, None) => return core::cmp::Ordering::Equal,
        }
    }
}

/// Case-insensitive `[char]` comparison against a lowercase table entry.
#[inline]
fn compare_chars_to_str(word: &[char], entry: &str) -> core::cmp::Ordering {
    let mut si = entry.chars();
    for &c in word {
        match si.next() {
            Some(sc) => match c.to_ascii_lowercase().cmp(&sc) {
                core::cmp::Ordering::Equal => continue,
                ord => return ord,
            },
            None => return core::cmp::Ordering::Greater,
        }
    }
    if si.next().is_some() {
        core::cmp::Ordering::Less
    } else {
        core::cmp::Ordering::Equal
    }
}

/// Returns `true` when `word` (a rendered word) is one of the curated English
/// function words.
///
/// Only called at word boundaries (space/commit), never per keystroke.
#[inline]
pub fn is_english_function_word(word: &str) -> bool {
    !word.is_empty() && contains_str(ENGLISH_FUNCTION_WORDS, word)
}

/// Returns `true` when `word` (the raw keystrokes of the current word) is one
/// of the always-English tech words.
///
/// Only called at word boundaries (space/commit), never per keystroke.
#[inline]
pub fn is_always_english_tech_word(word: &[char]) -> bool {
    !word.is_empty() && contains_chars(ALWAYS_ENGLISH_TECH_WORDS, word)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(w: &str) -> Vec<char> {
        w.chars().collect()
    }

    #[test]
    fn tables_are_sorted_and_lowercase() {
        for table in [ENGLISH_FUNCTION_WORDS, ALWAYS_ENGLISH_TECH_WORDS] {
            for w in table {
                assert!(w.chars().all(|c| !c.is_ascii_uppercase()), "{w}");
            }
            assert!(table.windows(2).all(|w| w[0] < w[1]));
        }
    }

    #[test]
    fn function_word_lookup() {
        for w in ["the", "my", "this", "their", "was", "I"] {
            assert!(is_english_function_word(w), "{w}");
        }
        for w in ["main", "freezes", "car", "hello", ""] {
            assert!(!is_english_function_word(w), "{w}");
        }
    }

    #[test]
    fn always_english_tech_word_lookup() {
        for w in ["test", "tests", "list", "host", "cost", "boost", "TEST"] {
            assert!(is_always_english_tech_word(&chars(w)), "{w}");
        }
        for w in ["tets", "rest", "tester", ""] {
            assert!(!is_always_english_tech_word(&chars(w)), "{w}");
        }
    }
}
