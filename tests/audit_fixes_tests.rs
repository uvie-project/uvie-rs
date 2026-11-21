//! Locked expectations for the perf-accuracy-audit fixes: VNI modifier digits
//! across a coda, Telex `w` after the "gi" onset, and ALL-CAPS Vietnamese
//! runs surviving the acronym restore.

use uvie::InputMethod;
use uvie::UltraFastViEngine;
use uvie::diff::Diffable;

fn apply(screen: &mut String, bs: usize, suffix: &str) {
    let sc: Vec<char> = screen.chars().collect();
    *screen = sc[..sc.len().saturating_sub(bs)].iter().collect::<String>();
    screen.push_str(suffix);
}

fn screen_after(method: InputMethod, keys: &str) -> String {
    let mut e = UltraFastViEngine::new();
    e.set_input_method(method);
    let mut screen = String::new();
    for ch in keys.chars() {
        let (bs, suffix) = e.feed_diff(ch);
        apply(&mut screen, bs, suffix);
    }
    screen
}

#[track_caller]
fn check(method: InputMethod, keys: &str, want: &str) {
    assert_eq!(screen_after(method, keys), want, "keys: {keys:?}");
}

// ---- Bug A: VNI modifier digits work across a coda ----

#[test]
fn vni_circumflex_after_coda() {
    let v = InputMethod::Vni;
    check(v, "bien6", "biên");
    check(v, "tieng61 ", "tiếng ");
    check(v, "tie6ng1 ", "tiếng ");
    check(v, "thuoc61 ", "thuốc ");
    check(v, "muon61 ", "muốn ");
    check(v, "khuyen61 ", "khuyến ");
    check(v, "TIENG61 ", "TIẾNG ");
}

#[test]
fn vni_horn_after_coda() {
    let v = InputMethod::Vni;
    check(v, "chu7o7ng", "chương");
    check(v, "chuo7ng ", "chương ");
    check(v, "nguoi72 ", "người ");
    check(v, "d9u7o7c5 ", "được ");
    check(v, "d9u7o7ng2 ", "đường ");
    check(v, "duoc975 ", "được ");
    check(v, "tung7", "tưng");
    check(v, "son7", "sơn");
    check(v, "mua7", "mưa");
    check(v, "luu7", "lưu");
    check(v, "gui73", "gửi");
    check(v, "oi7", "ơi");
    check(v, "ruou75", "rượu");
    check(v, "quo7", "quơ");
    check(v, "thuo73", "thuở");
    check(v, "huo7", "huơ");
    check(v, "u7o7", "ươ");
}

#[test]
fn vni_breve_after_coda() {
    let v = InputMethod::Vni;
    check(v, "an8", "ăn");
    check(v, "hoac85", "hoặc");
    check(v, "quan8", "quăn");
    check(v, "dat981", "đắt");
}

#[test]
fn vni_dd_from_anywhere() {
    let v = InputMethod::Vni;
    check(v, "duoc9", "đuoc");
    check(v, "dam96", "đâm");
    check(v, "duong972 ", "đường ");
}

#[test]
fn vni_literals_and_tones_unchanged() {
    let v = InputMethod::Vni;
    check(v, "2024 ", "2024 ");
    check(v, "covid19", "covid19");
    check(v, "a1 ", "á ");
}

#[test]
fn vni_double_digit_cancel_keeps_literal() {
    let v = InputMethod::Vni;
    check(v, "a66", "a6");
    check(v, "e66", "e6");
    check(v, "o66", "o6");
    check(v, "o77", "o7");
    check(v, "u77", "u7");
    check(v, "a88", "a8");
    check(v, "d99", "d9");
    check(v, "uo77", "uo7");
    check(v, "tie6ng6", "tieng6");
    check(v, "a66n ", "a6n ");
    check(v, "d99a ", "d9a ");
    // The redundant 7 on a resolved [ư,ơ] is consumed, a further 7 cancels.
    check(v, "u7o7", "ươ");
    check(v, "u7o77", "uo7");
    check(v, "tu7o7ng2 ", "tường ");
    check(v, "nu7o7c1 ", "nước ");
    check(v, "d9u7o7c5 ", "được ");
    // Locked app expectations: a digit undoing a transform made by another
    // key just vanishes ("aa8" -> "ă", "dd9" -> "d").
    check(v, "aa8", "ă");
    check(v, "dd9", "d");
    check(v, "aa6", "a");
    check(v, "u7ow1", "u7ow1");
}

#[test]
fn vni_backspace_sanity() {
    let mut e = UltraFastViEngine::new();
    e.set_input_method(InputMethod::Vni);
    let mut screen = String::new();
    for ch in "tieng61".chars() {
        let (bs, suffix) = e.feed_diff(ch);
        apply(&mut screen, bs, suffix);
    }
    for _ in 0..3 {
        let (bs, suffix) = e.backspace_diff();
        apply(&mut screen, bs, suffix);
    }
    for ch in "g61".chars() {
        let (bs, suffix) = e.feed_diff(ch);
        apply(&mut screen, bs, suffix);
    }
    assert_eq!(screen, "tiếng");
}

// ---- Bug B: Telex w shorthand after the "gi" onset ----

#[test]
fn telex_w_after_gi_onset() {
    let t = InputMethod::Telex;
    check(t, "giwx", "giữ");
    check(t, "giwax", "giữa");
    check(t, "giwowngf", "giường");
    check(t, "giwowng", "giương");
    check(t, "giwtj", "giựt");
}

// ---- Bug C: ALL-CAPS Vietnamese runs are not restored ----

#[test]
fn caps_lock_vietnamese_run() {
    let t = InputMethod::Telex;
    check(t, "TIEENGS VIEEJT HUEEJ ", "TIẾNG VIỆT HUỆ ");
    check(t, "TIEENGS VIEEJT TIMS ", "TIẾNG VIỆT TÍM ");
}

#[test]
fn acronyms_still_restore() {
    let t = InputMethod::Telex;
    check(t, "JIRA ", "JIRA ");
    check(t, "dufng JIRA ", "dùng JIRA ");
}
