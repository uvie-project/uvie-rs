// ===========================================================================
// Bug #11: punctuation `/` `\` `-` `_` etc. must act as word boundaries so
// Vietnamese transforms work after them. Previously only `.,!?;:"'()[]{} \n\r\t`
// were boundaries, so `/duowcs` produced `/duowcs` (literal) instead of
// `/dước` because the leading `/` was pushed as a consonant into the onset,
// failing `is_legal_onset` and silently disabling `w` horn application.
// ===========================================================================

#[cfg(test)]
mod word_boundary_punctuation_tests {
    use uvie::UltraFastViEngine;
    use uvie::diff::Diffable;

    fn type_diff(e: &mut UltraFastViEngine, s: &str) -> String {
        let mut screen = String::new();
        for ch in s.chars() {
            let (bs, suffix) = e.feed_diff(ch);
            let sc: Vec<char> = screen.chars().collect();
            screen = sc[..sc.len().saturating_sub(bs)].iter().collect::<String>();
            screen.push_str(suffix);
        }
        screen
    }

    #[test]
    fn slash_then_duowcs_produces_duoc_with_sac() {
        let mut e = UltraFastViEngine::new();
        // `duowcs` = d + ươ + c + sắc → dước (no đ because user typed `d` once)
        assert_eq!(type_diff(&mut e, "/duowcs"), "/dước");
    }

    #[test]
    fn slash_then_dduowcs_produces_duoc_with_d_and_sac() {
        let mut e = UltraFastViEngine::new();
        // `dduowcs` = đ + ươ + c + sắc → đước
        assert_eq!(type_diff(&mut e, "/dduowcs"), "/đước");
    }

    #[test]
    fn backslash_boundary() {
        let mut e = UltraFastViEngine::new();
        assert_eq!(type_diff(&mut e, "\\duowcs"), "\\dước");
    }

    #[test]
    fn dash_boundary() {
        let mut e = UltraFastViEngine::new();
        // "dauw": "ău" is not a legal rime, so the w is consumed and the
        // letters pass through as "dau". The point of this test is the dash
        // acting as a word boundary between "tiếng" and the next word.
        assert_eq!(type_diff(&mut e, "tieengs-dauw"), "tiếng-dau");
    }

    #[test]
    fn underscore_boundary() {
        let mut e = UltraFastViEngine::new();
        // `bee` → bê (ee→ê), then `f` is huyền tone → bề. The engine processes
        // `bee` as a valid Vietnamese syllable; `test` also gets V-C-V split.
        assert_eq!(type_diff(&mut e, "test_beef"), "tét_bề");
    }

    #[test]
    fn url_passthrough() {
        let mut e = UltraFastViEngine::new();
        // URL with `://` — every punctuation is a boundary; letters between are
        // English passthrough (no valid VN syllable triggers a transform).
        assert_eq!(type_diff(&mut e, "http://abc.com"), "http://abc.com");
    }

    #[test]
    fn at_sign_boundary() {
        let mut e = UltraFastViEngine::new();
        e.set_modern_orthography(true);
        // `user` is a common English word (top 5k) whose Vietnamese render
        // (`uể`) is a real word but needs a non-adjacent ê, so the English
        // reading wins. Then `@` boundary, then `hoas` → `hoá`.
        assert_eq!(type_diff(&mut e, "user@hoas"), "user@hoá");
    }

    #[test]
    fn hash_boundary() {
        let mut e = UltraFastViEngine::new();
        // `cai` → `cái` (i gets sắc tone). The engine processes `caii` as
        // `cái` + `i` (V-C-V split), but `i` alone is valid so it stays.
        // Just verify `#` acts as boundary so `cai` after it gets transformed.
        let result = type_diff(&mut e, "#cais");
        assert_eq!(result, "#cái");
    }

    #[test]
    fn slash_alone_passthrough() {
        let mut e = UltraFastViEngine::new();
        assert_eq!(type_diff(&mut e, "/"), "/");
        assert_eq!(type_diff(&mut e, "//"), "//");
    }

    #[test]
    fn vni_digit_not_boundary() {
        // Digits must NOT be word boundaries — VNI uses 0-9 as tone/modifier keys.
        let mut e = UltraFastViEngine::new();
        e.set_input_method(uvie::InputMethod::Vni);
        // `a6` = â, `1` = sắc → `ấ`
        assert_eq!(type_diff(&mut e, "a61"), "ấ");
    }

    #[test]
    fn precomposed_vietnamese_not_boundary() {
        // Non-ASCII Vietnamese chars must NOT be treated as boundaries — they
        // are decomposed by `feed()` and flow through the normal path.
        let mut e = UltraFastViEngine::new();
        // `ấ` decomposes to a + ^ + sắc; feeding it should not clear state.
        assert_eq!(type_diff(&mut e, "ấ"), "ấ");
    }
}
