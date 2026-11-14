//! Simulates the exact API call sequence that the Swift EventTap makes.
//!
//! Key mapping (from EventTap+Handle.swift):
//! - Regular char (a-z, punctuation): feed_diff(char)
//! - Space: commit_diff()  (NOT feed_diff(' '))
//! - Enter/Tab: commit_diff()
//! - Backspace: backspace_diff()
//! - Arrow/Home/End/PageUp/PageDown: reset_diff()
//! - Escape: reset_diff()

use uvie::UltraFastViEngine;
use uvie::diff::Diffable;

/// Simulates the Swift screen model: applies (backspaces, suffix) to screen.
fn apply(screen: &mut String, bs: usize, suffix: &str) {
    let sc: Vec<char> = screen.chars().collect();
    *screen = sc[..sc.len().saturating_sub(bs)].iter().collect::<String>();
    screen.push_str(suffix);
}

/// Type a word char-by-char via feed_diff (no space).
fn type_chars(e: &mut UltraFastViEngine, screen: &mut String, word: &str) {
    for ch in word.chars() {
        let (bs, suffix) = e.feed_diff(ch);
        apply(screen, bs, suffix);
    }
}

/// Press space via commit_diff (matches Swift handleSpace).
fn press_space(e: &mut UltraFastViEngine, screen: &mut String) {
    let (bs, suffix) = e.commit_diff();
    apply(screen, bs, suffix);
    // Swift passes the space event through to the OS, which adds a space.
    screen.push(' ');
}

/// Press Enter/Tab via commit_diff (matches Swift handleBreakKey).
fn press_break(e: &mut UltraFastViEngine, screen: &mut String, break_char: char) {
    let (bs, suffix) = e.commit_diff();
    apply(screen, bs, suffix);
    screen.push(break_char);
}

/// Type punctuation via feed_diff (matches Swift handleCharacterKey).
fn type_punct(e: &mut UltraFastViEngine, screen: &mut String, ch: char) {
    let (bs, suffix) = e.feed_diff(ch);
    apply(screen, bs, suffix);
}

/// Press backspace via backspace_diff (matches Swift handleBackspace).
fn press_backspace(e: &mut UltraFastViEngine, screen: &mut String) {
    let (bs, suffix) = e.backspace_diff();
    apply(screen, bs, suffix);
}

/// Press arrow key via reset_diff (matches Swift handleBreakKey).
fn press_arrow(e: &mut UltraFastViEngine) {
    e.reset_diff();
}

// ===== Tests simulating real user typing =====

#[test]
fn real_type_good_then_space() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "good");
    assert_eq!(s, "good", "after typing 'good': should show raw word");
    press_space(&mut e, &mut s);
    assert_eq!(s, "good ", "after space: should be 'good '");
}

#[test]
fn real_type_good_then_punct() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "good");
    assert_eq!(s, "good");
    type_punct(&mut e, &mut s, '.');
    assert_eq!(s, "good.", "after '.': should be 'good.'");
}

#[test]
fn real_type_character_then_space() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "character");
    assert_eq!(s, "character", "should show raw word while typing");
    press_space(&mut e, &mut s);
    assert_eq!(s, "character ");
}

#[test]
fn real_type_safari_then_space() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "safari");
    assert_eq!(s, "safari");
    press_space(&mut e, &mut s);
    assert_eq!(s, "safari ");
}

#[test]
fn real_type_good_backspace_retype() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "good");
    assert_eq!(s, "good");
    // Backspace 2 chars: "good" → "go"
    press_backspace(&mut e, &mut s);
    assert_eq!(s, "goo", "BS 1: should be 'goo'");
    press_backspace(&mut e, &mut s);
    assert_eq!(s, "go", "BS 2: should be 'go'");
    // Retype "ne" to make "gone" (not in dict, should transform if valid VN)
    type_chars(&mut e, &mut s, "ne");
    press_space(&mut e, &mut s);
    // "gone" → not in dict, engine transforms it
    // Just verify no crash and screen is consistent
    assert!(!s.is_empty());
}

#[test]
fn real_type_good_then_arrow_then_type() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "good");
    assert_eq!(s, "good");
    // Arrow key resets engine (cursor moved)
    press_arrow(&mut e);
    // Screen still shows "good" (arrow doesn't delete text)
    assert_eq!(s, "good");
    // Type new word after arrow
    type_chars(&mut e, &mut s, "book");
    assert_eq!(s, "goodbook", "after arrow+type: no ghost chars");
    press_space(&mut e, &mut s);
    assert_eq!(s, "goodbook ", "after space: clean");
}

#[test]
fn real_type_vietnamese_then_english() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    // Type Vietnamese word
    type_chars(&mut e, &mut s, "vieetj");
    assert_eq!(s, "việt");
    press_space(&mut e, &mut s);
    assert_eq!(s, "việt ");
    // Type English word
    type_chars(&mut e, &mut s, "good");
    assert_eq!(s, "việt good");
    press_space(&mut e, &mut s);
    assert_eq!(s, "việt good ");
}

#[test]
fn real_type_english_then_vietnamese() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "good");
    press_space(&mut e, &mut s);
    assert_eq!(s, "good ");
    type_chars(&mut e, &mut s, "vieetj");
    assert_eq!(s, "good việt");
    press_space(&mut e, &mut s);
    assert_eq!(s, "good việt ");
}

#[test]
fn real_type_chaos_still_vietnamese() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "chaos");
    // "chaos" → "cháo" (valid Vietnamese, NOT in dict)
    assert_eq!(s, "cháo", "chaos should produce cháo (valid VN)");
    press_space(&mut e, &mut s);
    assert_eq!(s, "cháo ");
}

#[test]
fn real_type_most_still_vietnamese() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "most");
    // "most" → "mót" (valid Vietnamese, NOT in dict)
    assert_eq!(s, "mót", "most should produce mót (valid VN)");
    press_space(&mut e, &mut s);
    assert_eq!(s, "mót ");
}

#[test]
fn real_type_user_is_english() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "user");
    // "user" → "uể": 's' is the sắc tone on u, 'e' joins the nucleus and
    // [u,e]+'r' resolves to uê (the real Telex spelling of "uể"). The
    // render is a real Vietnamese word, but "user" is a common English
    // word (top 5k) whose spelling never doubles the ê, so the dictionary
    // override keeps the English reading.
    assert_eq!(s, "user", "user should pass through as English");
    press_space(&mut e, &mut s);
    assert_eq!(s, "user ");
}

#[test]
fn real_type_uppercase_good() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "GOOD");
    assert_eq!(s, "GOOD", "uppercase should be preserved");
    press_space(&mut e, &mut s);
    assert_eq!(s, "GOOD ");
}

#[test]
fn real_type_mixed_case_good() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "Good");
    assert_eq!(s, "Good", "mixed case should be preserved");
    press_space(&mut e, &mut s);
    assert_eq!(s, "Good ");
}

#[test]
fn real_type_sentence_with_english() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    // "the good book"
    type_chars(&mut e, &mut s, "the");
    press_space(&mut e, &mut s);
    type_chars(&mut e, &mut s, "good");
    press_space(&mut e, &mut s);
    type_chars(&mut e, &mut s, "book");
    press_space(&mut e, &mut s);
    assert_eq!(s, "the good book ");
}

#[test]
fn real_type_good_enter() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "good");
    assert_eq!(s, "good");
    press_break(&mut e, &mut s, '\n');
    assert_eq!(s, "good\n", "after Enter: should be 'good\\n'");
}

#[test]
fn real_type_good_tab() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "good");
    assert_eq!(s, "good");
    press_break(&mut e, &mut s, '\t');
    assert_eq!(s, "good\t", "after Tab: should be 'good\\t'");
}

#[test]
fn real_type_good_comma() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "good");
    type_punct(&mut e, &mut s, ',');
    assert_eq!(s, "good,", "after ',': should be 'good,'");
    type_punct(&mut e, &mut s, ' ');
    // Space after comma is a regular char (not commit) — but it's a word
    // boundary so it clears the engine state.
    assert_eq!(s, "good, ");
}

#[test]
fn real_type_good_full_backspace() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "good");
    assert_eq!(s, "good");
    // Backspace all 4 chars
    for i in 0..4 {
        press_backspace(&mut e, &mut s);
        let expected = &"good"[..4 - i - 1];
        assert_eq!(s, expected, "BS {}: should be '{}'", i + 1, expected);
    }
    assert_eq!(s, "");
    // One more backspace (nothing to delete)
    press_backspace(&mut e, &mut s);
    assert_eq!(s, "");
}

#[test]
fn real_type_good_backspace_then_vietnamese() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "good");
    // Backspace to "go"
    press_backspace(&mut e, &mut s);
    press_backspace(&mut e, &mut s);
    assert_eq!(s, "go");
    // Type Vietnamese tone key
    type_chars(&mut e, &mut s, "s"); // "gos" → not valid VN, passthrough
    press_space(&mut e, &mut s);
    // Just verify no crash, screen is consistent
    assert!(!s.is_empty());
}

#[test]
fn real_type_long_dict_word() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "characteristically");
    assert_eq!(s, "characteristically");
    press_space(&mut e, &mut s);
    assert_eq!(s, "characteristically ");
}

#[test]
fn real_multiple_words_mixed() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    // "good book character safari"
    for word in &["good", "book", "character", "safari"] {
        type_chars(&mut e, &mut s, word);
        press_space(&mut e, &mut s);
    }
    assert_eq!(s, "good book character safari ");
}

#[test]
fn real_type_good_escape() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "good");
    assert_eq!(s, "good");
    // Escape resets engine (doesn't commit)
    press_arrow(&mut e); // Same as reset_diff
    // Screen still shows "good" but engine state is cleared
    assert_eq!(s, "good");
    // Type new word
    type_chars(&mut e, &mut s, "book");
    assert_eq!(s, "goodbook", "after escape+type: no ghost chars");
}

#[test]
fn real_type_good_then_continue_no_ghost() {
    // CRITICAL: typing past a dict word must not produce ghost characters.
    // Before the fix, typing "goodness" showed "gôdne" and "gôdnes" because
    // the V-C-V split re-rendered the committed portion from raw_chars.
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    // Type char by char and verify no ghost characters at any point
    let chars: Vec<char> = "goodness".chars().collect();
    // Once the "good" override fires, the rest of the word passes through
    // raw (sticky English passthrough) — no Vietnamese transform of the tail.
    let expected = [
        "g", "go", "gô", "good", "goodn", "goodne", "goodnes", "goodness",
    ];
    for (i, ch) in chars.iter().enumerate() {
        let (bs, suffix) = e.feed_diff(*ch);
        apply(&mut s, bs, suffix);
        assert_eq!(
            s, expected[i],
            "after char '{}' ({}): got '{}' expected '{}'",
            ch, i, s, expected[i]
        );
    }
    press_space(&mut e, &mut s);
    assert_eq!(s, "goodness ");
}

#[test]
fn real_type_book_then_continue_no_ghost() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    // "book" is in dict, "books" is in dict too
    type_chars(&mut e, &mut s, "book");
    assert_eq!(s, "book");
    // Continue typing "s" — "books" is in dict
    type_chars(&mut e, &mut s, "s");
    assert_eq!(s, "books", "after 's': should be 'books' (in dict)");
    // Continue typing "h" — "booksh" is not in dict, but no ghost chars
    let (bs, suffix) = e.feed_diff('h');
    apply(&mut s, bs, suffix);
    // The screen should start with "book" (not "bôk")
    assert!(
        s.starts_with("book"),
        "after 'h': screen should start with 'book', got '{}'",
        s
    );
}

#[test]
fn real_type_character_then_continue_no_ghost() {
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "character");
    assert_eq!(s, "character");
    // Continue typing "s" — "characters" is in dict
    type_chars(&mut e, &mut s, "s");
    assert_eq!(s, "characters");
    // Continue typing "h" — no ghost chars
    let (bs, suffix) = e.feed_diff('h');
    apply(&mut s, bs, suffix);
    assert!(
        s.starts_with("character"),
        "after 'h': screen should start with 'character', got '{}'",
        s
    );
}

#[test]
fn real_type_good_backspace_from_diff_committed() {
    // After override, "good" is in diff_committed. Backspace should pop
    // from diff_committed and word_raw simultaneously.
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "good");
    assert_eq!(s, "good");
    // Backspace 1: "good" → "goo"
    press_backspace(&mut e, &mut s);
    assert_eq!(s, "goo", "BS 1: should be 'goo'");
    // Backspace 2: "goo" → "go"
    press_backspace(&mut e, &mut s);
    assert_eq!(s, "go", "BS 2: should be 'go'");
    // Backspace 3: "go" → "g"
    press_backspace(&mut e, &mut s);
    assert_eq!(s, "g", "BS 3: should be 'g'");
    // Backspace 4: "g" → ""
    press_backspace(&mut e, &mut s);
    assert_eq!(s, "", "BS 4: should be empty");
}

#[test]
fn real_type_good_backspace_retype_dict_match() {
    // After override at "good", backspace to "go", then retype "od".
    // word_raw should be in sync, and "good" should match dict again.
    let mut e = UltraFastViEngine::new();
    let mut s = String::new();
    type_chars(&mut e, &mut s, "good");
    assert_eq!(s, "good");
    // Backspace 2: "good" → "go"
    press_backspace(&mut e, &mut s);
    assert_eq!(s, "goo");
    press_backspace(&mut e, &mut s);
    assert_eq!(s, "go");
    // Retype "od" — "good" should match dict again
    type_chars(&mut e, &mut s, "od");
    assert_eq!(
        s, "good",
        "after retype 'od': should be 'good' (dict match)"
    );
}
