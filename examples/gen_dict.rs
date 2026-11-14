//! English override dictionary generator.
//!
//! Rule (per AGENTS.md design): include an English word iff its Telex
//! feed_diff render differs from the raw word AND the rendered text is NOT a
//! real Vietnamese word on its own (from the 22k word list, tones kept).
//!
//! A Vietnamese token typed without spaces is a single syllable, so a render
//! that merely *decomposes* into several real words at consonant onsets
//! ("after" → "à"+"tẻ", "master" → "má"+"tẻ") is not Vietnamese intent —
//! that multi-word exclusion is gone.
//!
//! One exception re-includes a render that is a single real word: a common
//! English word (top ~20k of the frequency corpus) whose Vietnamese reading
//! needs a NON-adjacent doubled letter ("reets", "theem", "ddefend") while
//! the English spelling does not contain that adjacent pair ("reset",
//! "theme", "defend"). Vietnamese typists type the canonical doubled form,
//! so the English reading must win for those.
use std::collections::HashSet;
use uvie::UltraFastViEngine;
use uvie::diff::Diffable;

// Override is disabled on the generator engine so we see the raw transform
// the dictionary is meant to fix (feed_diff applies the override per-keystroke).
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

/// Strip Vietnamese tone marks but keep vowel-quality marks (ă â ê ô ơ ư đ).
fn strip_tones(c: char) -> char {
    match c {
        'á' | 'à' | 'ả' | 'ã' | 'ạ' => 'a',
        'ấ' | 'ầ' | 'ẩ' | 'ẫ' | 'ậ' => 'â',
        'ắ' | 'ằ' | 'ẳ' | 'ẵ' | 'ặ' => 'ă',
        'é' | 'è' | 'ẻ' | 'ẽ' | 'ẹ' => 'e',
        'ế' | 'ề' | 'ể' | 'ễ' | 'ệ' => 'ê',
        'í' | 'ì' | 'ỉ' | 'ĩ' | 'ị' => 'i',
        'ó' | 'ò' | 'ỏ' | 'õ' | 'ọ' => 'o',
        'ố' | 'ồ' | 'ổ' | 'ỗ' | 'ộ' => 'ô',
        'ớ' | 'ờ' | 'ở' | 'ỡ' | 'ợ' => 'ơ',
        'ú' | 'ù' | 'ủ' | 'ũ' | 'ụ' => 'u',
        'ứ' | 'ừ' | 'ử' | 'ữ' | 'ự' => 'ư',
        'ý' | 'ỳ' | 'ỷ' | 'ỹ' | 'ỵ' => 'y',
        'Á' | 'À' | 'Ả' | 'Ã' | 'Ạ' => 'A',
        'Ấ' | 'Ầ' | 'Ẩ' | 'Ẫ' | 'Ậ' => 'Â',
        'Ắ' | 'Ằ' | 'Ẳ' | 'Ẵ' | 'Ặ' => 'Ă',
        'É' | 'È' | 'Ẻ' | 'Ẽ' | 'Ẹ' => 'E',
        'Ế' | 'Ề' | 'Ể' | 'Ễ' | 'Ệ' => 'Ê',
        'Í' | 'Ì' | 'Ỉ' | 'Ĩ' | 'Ị' => 'I',
        'Ó' | 'Ò' | 'Ỏ' | 'Õ' | 'Ọ' => 'O',
        'Ố' | 'Ồ' | 'Ổ' | 'Ỗ' | 'Ộ' => 'Ô',
        'Ớ' | 'Ờ' | 'Ở' | 'Ỡ' | 'Ợ' => 'Ơ',
        'Ú' | 'Ù' | 'Ủ' | 'Ũ' | 'Ụ' => 'U',
        'Ứ' | 'Ừ' | 'Ử' | 'Ữ' | 'Ự' => 'Ư',
        'Ý' | 'Ỳ' | 'Ỷ' | 'Ỹ' | 'Ỵ' => 'Y',
        c => c,
    }
}

/// Does the render need a doubled letter the raw English word does not
/// contain adjacently? The Telex spellings are "aa" (â), "ee" (ê), "oo"
/// (ô) and "dd" (đ); a word that never doubles the letter can only reach
/// the render through the non-canonical order ("e…s…e"), so the English
/// reading wins.
fn needs_nonadjacent_double(word: &str, render: &str) -> bool {
    let base: String = render.chars().map(strip_tones).collect();
    [('â', "aa"), ('ê', "ee"), ('ô', "oo")]
        .iter()
        .any(|&(v, pair)| base.contains(v) && !word.contains(pair))
        || (base.contains('đ') && !word.contains("dd"))
}

fn main() {
    // Set of real Vietnamese WORDS (tones kept) — every whitespace-separated
    // token of the 22k list. A Vietnamese token typed without spaces is one
    // syllable, so only a whole-token match counts as Vietnamese intent.
    let vi_data = include_str!("../tests/data/vietnamese_22k.txt");
    let mut words: HashSet<String> = HashSet::new();
    for line in vi_data.lines() {
        for tok in line.split_whitespace() {
            let lw: String = tok.chars().map(|c| c.to_ascii_lowercase()).collect();
            if lw.chars().all(|c| c.is_alphabetic()) {
                words.insert(lw);
            }
        }
    }
    println!("word set: {}", words.len());

    let en_data = include_str!("../tests/data/english_100k.txt");

    // Prefix-shadow drop: a dictionary word that is a prefix of a real
    // Telex keystroke sequence fires the per-keystroke override mid-word
    // and hijacks Vietnamese typing ("tojo" prefixes "tojot" → "tột").
    // Rare English loses to Vietnamese intent; common English (top ~20k
    // of the frequency-ordered corpus) wins — the same balance the
    // curated list struck ("dust", "data").
    let pairs_data = include_str!("../tests/data/vietnamese_telex_pairs.txt");
    let mut pair_inputs: Vec<&str> = pairs_data
        .lines()
        .filter_map(|l| l.split('\t').next())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    pair_inputs.sort();
    let top20k: HashSet<&str> = en_data.lines().take(20_000).map(|l| l.trim()).collect();
    // Stricter gate for the doubled-tone-key class: below top ~10k the
    // cancel mechanics win over the English word.
    let top10k: HashSet<&str> = en_data.lines().take(10_000).map(|l| l.trim()).collect();
    let top5k: HashSet<&str> = en_data.lines().take(5_000).map(|l| l.trim()).collect();
    let shadows_pair = |w: &str| -> bool {
        // First pair input >= w; if it starts with w, w is a prefix.
        let idx = pair_inputs.partition_point(|p| *p < w);
        idx < pair_inputs.len() && pair_inputs[idx].starts_with(w)
    };
    // Words ending in a doubled Telex tone key (ss/ff/rr/xx/jj) collide
    // with the standard double-same-tone-key cancel: UniKey collapses the
    // pair and restores the raw consonant (cass→cas, timff→timf). Keeping
    // the word in the dict would steal the only way to type the collapsed
    // form, so only genuinely common English words earn the protection.
    let ends_doubled_tone = |w: &str| -> bool {
        let b = w.as_bytes();
        b.len() >= 2
            && b[b.len() - 1] == b[b.len() - 2]
            && matches!(b[b.len() - 1], b's' | b'f' | b'r' | b'x' | b'j')
    };

    let mut dict: Vec<String> = Vec::new();
    let mut n_passthrough = 0;
    let mut n_valid_vi = 0;
    let mut n_reincluded = 0;
    let mut n_shadowed = 0;
    let mut n_doubled = 0;
    for line in en_data.lines() {
        let word = line.trim();
        if word.len() < 4 || !word.chars().all(|c| c.is_ascii_lowercase()) {
            continue;
        }
        let mut e = UltraFastViEngine::new();
        e.set_modern_orthography(false);
        e.set_english_override(false);
        let rendered = type_word(&mut e, word);
        if rendered == word {
            n_passthrough += 1;
            continue;
        }
        let lw_render: String = rendered.chars().map(|c| c.to_ascii_lowercase()).collect();
        if words.contains(&lw_render) {
            // The render is a real Vietnamese word: Vietnamese intent wins,
            // unless common English only reaches it through a non-canonical
            // keystroke order.
            if !(top20k.contains(word) && needs_nonadjacent_double(word, &lw_render)) {
                n_valid_vi += 1;
                continue;
            }
            n_reincluded += 1;
        }
        if shadows_pair(word) && !top20k.contains(word) {
            n_shadowed += 1;
            continue;
        }
        if ends_doubled_tone(word) && !top10k.contains(word) {
            n_doubled += 1;
            continue;
        }
        dict.push(word.to_string());
    }
    dict.sort();
    dict.dedup();

    // Words currently shipped in src/tables/english.rs — the new list is a
    // union with them so no already-protected word is lost.
    let cur: HashSet<String> = include_str!("../src/tables/english.rs")
        .lines()
        .filter(|l| l.starts_with("    \""))
        .filter_map(|l| {
            l.trim()
                .trim_end_matches(',')
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
        })
        .map(|s| s.to_string())
        .collect();
    println!(
        "passthrough: {}, valid-vi(excluded): {}, re-included: {}, shadowed: {}, doubled-tone: {}, new list: {}, current: {}",
        n_passthrough,
        n_valid_vi,
        n_reincluded,
        n_shadowed,
        n_doubled,
        dict.len(),
        cur.len()
    );

    let new: HashSet<String> = dict.iter().cloned().collect();
    let mut merged: Vec<String> = cur.union(&new).cloned().collect();
    merged.sort();
    merged.dedup();
    println!("merged dict: {}", merged.len());

    let mut added: Vec<&String> = new.difference(&cur).collect();
    added.sort();
    println!("newly added: {}", added.len());
    let mut added_top5k: Vec<&str> = added
        .iter()
        .map(|w| w.as_str())
        .filter(|w| top5k.contains(w))
        .collect();
    added_top5k.sort();
    println!("newly added in top 5k: {:?}", added_top5k);

    let path = std::env::args().nth(1).unwrap_or_else(|| {
        "/private/tmp/claude-501/-Users-thupham-Documents-Workspace/96c58022-f10a-4dec-b879-dca7136e0a85/scratchpad/new_dict.txt".to_string()
    });
    use std::io::Write;
    let mut f = std::fs::File::create(&path).unwrap();
    for w in &merged {
        writeln!(f, "    \"{}\",", w).unwrap();
    }
    println!("wrote {} words to {}", merged.len(), path);
}
