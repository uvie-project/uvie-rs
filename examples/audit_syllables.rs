//! Exhaustive syllable audit: every distinct syllable of the 22k word list is
//! typed through `feed_diff` in every common keystroke order (Telex and VNI),
//! isolated and inside a Vietnamese sentence, lower/Title/UPPER case. The
//! screen is rebuilt from the diffs, so a mismatch is what the user would see.
//!
//! Usage: cargo run --release --example audit_syllables [-- <out_dir>]
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use uvie::diff::Diffable;
use uvie::{InputMethod, UltraFastViEngine};

const PLAIN: [char; 12] = ['a', 'ă', 'â', 'e', 'ê', 'i', 'o', 'ô', 'ơ', 'u', 'ư', 'y'];
const BASE: [char; 12] = ['a', 'a', 'a', 'e', 'e', 'i', 'o', 'o', 'o', 'u', 'u', 'y'];
/// 0 none, 1 circumflex, 2 horn/breve
const MODK: [u8; 12] = [0, 2, 1, 0, 1, 0, 0, 1, 2, 0, 2, 0];
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
const TELEX_TONE: [char; 6] = ['\0', 's', 'f', 'r', 'x', 'j'];
const VNI_TONE: [char; 6] = ['\0', '1', '2', '3', '4', '5'];

#[derive(Clone, Copy)]
struct Ch {
    /// index into PLAIN, or usize::MAX for a consonant
    v: usize,
    /// consonant char (lowercase; 'đ' kept)
    c: char,
    tone: usize,
}

fn decompose(syl: &str) -> Option<Vec<Ch>> {
    let mut out = Vec::new();
    for c in syl.chars() {
        let mut found = false;
        for (i, forms) in TONE_FORMS.iter().enumerate() {
            if let Some(t) = forms.iter().position(|&f| f == c) {
                out.push(Ch { v: i, c, tone: t });
                found = true;
                break;
            }
        }
        if !found {
            if !(c.is_ascii_lowercase() || c == 'đ') {
                return None;
            }
            out.push(Ch {
                v: usize::MAX,
                c,
                tone: 0,
            });
        }
    }
    Some(out)
}

/// (toneless base string, tone) - equal for two spellings that differ only in
/// which vowel carries the tone mark ("hoà" vs "hòa").
fn tone_normal(s: &str) -> (String, usize) {
    let mut base = String::new();
    let mut tone = 0;
    for c in s.chars() {
        let lc = c.to_lowercase().next().unwrap_or(c);
        let mut hit = false;
        for (i, forms) in TONE_FORMS.iter().enumerate() {
            if let Some(t) = forms.iter().position(|&f| f == lc) {
                base.push(PLAIN[i]);
                if t > 0 {
                    tone = t;
                }
                hit = true;
                break;
            }
        }
        if !hit {
            base.push(lc);
        }
    }
    (base, tone)
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Style {
    /// modifiers inline, tone at the end of the word ("dduowngf")
    TelexToneEnd,
    /// modifiers inline, tone right after the nucleus ("dduowfng")
    TelexToneAfterNucleus,
    /// modifiers inline, tone right after the toned vowel ("nguowfi")
    TelexToneInline,
    /// tone right after the toned vowel's BASE letter, then modifier ("dduofwng")
    TelexToneBeforeMod,
    /// modifiers after the whole word, then tone ("dduongwf", "tienges")
    TelexModsEnd,
    /// tone at the end, then modifiers ("dduongfw", "tiengse")
    TelexToneThenMods,
    /// "ươ" typed "uow" (one w), tone at the end ("dduowcj")
    TelexUoSingleW,
    /// ư typed with the bare "w" shorthand ("nhw", "dwowcj")
    TelexWShorthand,
    /// VNI: modifier digit inline, tone digit at the end ("d9u7o7ng2")
    VniToneEnd,
    /// VNI: all digits at the end ("duong972")
    VniAllEnd,
    /// VNI: tone digit right after the nucleus, modifiers inline
    VniToneAfterNucleus,
}

const ALL_STYLES: [Style; 11] = [
    Style::TelexToneEnd,
    Style::TelexToneAfterNucleus,
    Style::TelexToneInline,
    Style::TelexToneBeforeMod,
    Style::TelexModsEnd,
    Style::TelexToneThenMods,
    Style::TelexUoSingleW,
    Style::TelexWShorthand,
    Style::VniToneEnd,
    Style::VniAllEnd,
    Style::VniToneAfterNucleus,
];

fn is_vni(s: Style) -> bool {
    matches!(
        s,
        Style::VniToneEnd | Style::VniAllEnd | Style::VniToneAfterNucleus
    )
}

fn telex_mod(v: usize) -> Option<char> {
    match MODK[v] {
        1 => Some(BASE[v]),
        2 => Some('w'),
        _ => None,
    }
}

fn vni_mod(v: usize) -> Option<char> {
    match PLAIN[v] {
        'â' | 'ê' | 'ô' => Some('6'),
        'ơ' | 'ư' => Some('7'),
        'ă' => Some('8'),
        _ => None,
    }
}

/// Keystrokes for one syllable in one style; None when the style does not
/// apply (would equal another style or has no such spelling).
fn keys(chs: &[Ch], style: Style) -> Option<String> {
    let tone = chs.iter().map(|c| c.tone).max().unwrap_or(0);
    let last_vowel = chs.iter().rposition(|c| c.v != usize::MAX);
    let has_mod = chs.iter().any(|c| c.v != usize::MAX && MODK[c.v] != 0);
    let has_dd = chs.iter().any(|c| c.c == 'đ');
    let vni = is_vni(style);
    let tk = if vni {
        VNI_TONE[tone]
    } else {
        TELEX_TONE[tone]
    };
    let mut out = String::new();
    let mut end = String::new();
    match style {
        Style::TelexToneEnd
        | Style::TelexToneAfterNucleus
        | Style::TelexToneInline
        | Style::TelexToneBeforeMod
        | Style::VniToneEnd
        | Style::VniToneAfterNucleus => {
            if matches!(
                style,
                Style::TelexToneAfterNucleus
                    | Style::TelexToneInline
                    | Style::TelexToneBeforeMod
                    | Style::VniToneAfterNucleus
            ) && tone == 0
            {
                return None;
            }
            if style == Style::TelexToneBeforeMod
                && !chs
                    .iter()
                    .any(|c| c.tone > 0 && c.v != usize::MAX && MODK[c.v] != 0)
            {
                return None;
            }
            for (i, c) in chs.iter().enumerate() {
                if c.v == usize::MAX {
                    if c.c == 'đ' {
                        out.push_str(if vni { "d9" } else { "dd" });
                    } else {
                        out.push(c.c);
                    }
                    continue;
                }
                out.push(BASE[c.v]);
                if style == Style::TelexToneBeforeMod && c.tone > 0 {
                    out.push(tk);
                }
                let m = if vni { vni_mod(c.v) } else { telex_mod(c.v) };
                if let Some(m) = m {
                    out.push(m);
                }
                match style {
                    Style::TelexToneInline if c.tone > 0 => out.push(tk),
                    Style::TelexToneAfterNucleus | Style::VniToneAfterNucleus
                        if Some(i) == last_vowel =>
                    {
                        out.push(tk)
                    }
                    _ => {}
                }
            }
            if matches!(style, Style::TelexToneEnd | Style::VniToneEnd) && tone > 0 {
                end.push(tk);
            }
        }
        Style::TelexModsEnd | Style::TelexToneThenMods | Style::VniAllEnd => {
            if !has_mod && !(vni && has_dd) {
                return None;
            }
            let mut mods = String::new();
            for c in chs {
                if c.v == usize::MAX {
                    if c.c == 'đ' {
                        if vni {
                            out.push('d');
                            mods.push('9');
                        } else {
                            out.push_str("dd");
                        }
                    } else {
                        out.push(c.c);
                    }
                    continue;
                }
                out.push(BASE[c.v]);
                let m = if vni { vni_mod(c.v) } else { telex_mod(c.v) };
                if let Some(m) = m {
                    // one horn key covers "ươ"; one key per modifier kind
                    if !mods.contains(m) {
                        mods.push(m);
                    }
                }
            }
            if style == Style::TelexToneThenMods {
                if tone == 0 {
                    return None;
                }
                end.push(tk);
                end.push_str(&mods);
            } else {
                end.push_str(&mods);
                if tone > 0 {
                    end.push(tk);
                }
            }
        }
        Style::TelexUoSingleW | Style::TelexWShorthand => {
            // only syllables with ư
            if !chs.iter().any(|c| c.v != usize::MAX && PLAIN[c.v] == 'ư') {
                return None;
            }
            let mut i = 0;
            let mut any = false;
            while i < chs.len() {
                let c = chs[i];
                if c.v == usize::MAX {
                    if c.c == 'đ' {
                        out.push_str("dd");
                    } else {
                        out.push(c.c);
                    }
                    i += 1;
                    continue;
                }
                let is_uo = PLAIN[c.v] == 'ư'
                    && i + 1 < chs.len()
                    && chs[i + 1].v != usize::MAX
                    && PLAIN[chs[i + 1].v] == 'ơ';
                if style == Style::TelexUoSingleW {
                    if is_uo {
                        out.push_str("uow");
                        any = true;
                        i += 2;
                        continue;
                    }
                    out.push(BASE[c.v]);
                    if let Some(m) = telex_mod(c.v) {
                        out.push(m);
                    }
                } else if PLAIN[c.v] == 'ư' {
                    out.push('w');
                    any = true;
                } else {
                    out.push(BASE[c.v]);
                    if let Some(m) = telex_mod(c.v) {
                        out.push(m);
                    }
                }
                i += 1;
            }
            if !any {
                return None;
            }
            if tone > 0 {
                end.push(tk);
            }
        }
    }
    out.push_str(&end);
    Some(out)
}

fn apply(screen: &mut Vec<char>, bs: usize, suffix: &str) {
    let n = screen.len().saturating_sub(bs);
    screen.truncate(n);
    screen.extend(suffix.chars());
}

fn type_keys(method: InputMethod, text: &str) -> String {
    let mut e = UltraFastViEngine::new();
    e.set_input_method(method);
    let mut screen = Vec::new();
    for ch in text.chars() {
        let (bs, s) = e.feed_diff(ch);
        let s = s.to_string();
        apply(&mut screen, bs, &s);
    }
    screen.into_iter().collect()
}

fn case_keys(k: &str, mode: u8) -> String {
    match mode {
        1 => {
            let mut it = k.chars();
            let f = it.next().map(|c| c.to_ascii_uppercase());
            f.into_iter().chain(it).collect()
        }
        2 => k.to_ascii_uppercase(),
        _ => k.to_string(),
    }
}

fn case_word(w: &str, mode: u8) -> String {
    match mode {
        1 => {
            let mut it = w.chars();
            let f: String = it
                .next()
                .map(|c| c.to_uppercase().collect())
                .unwrap_or_default();
            f + it.as_str()
        }
        2 => w.to_uppercase(),
        _ => w.to_string(),
    }
}

fn main() {
    let out_dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/uvie_audit".into());
    std::fs::create_dir_all(&out_dir).unwrap();
    let words = std::fs::read_to_string("tests/data/vietnamese_22k.txt").unwrap();
    let mut syls: BTreeSet<String> = BTreeSet::new();
    for line in words.lines() {
        for w in line.split(|c: char| !c.is_alphabetic()) {
            let w = w.to_lowercase();
            if !w.is_empty() && decompose(&w).is_some() {
                syls.insert(w);
            }
        }
    }
    println!("syllables: {}", syls.len());

    // (style, context, case) -> (total, exact_fail, placement_only)
    let mut stats: BTreeMap<(Style, &str, u8), (usize, usize, usize)> = BTreeMap::new();
    let mut logs: BTreeMap<(Style, &str, u8), String> = BTreeMap::new();
    for syl in &syls {
        let chs = decompose(syl).unwrap();
        for style in ALL_STYLES {
            let Some(k) = keys(&chs, style) else { continue };
            let method = if is_vni(style) {
                InputMethod::Vni
            } else {
                InputMethod::Telex
            };
            for case in 0u8..3 {
                // Title/UPPER only on the two canonical styles to keep it readable
                if case > 0
                    && !matches!(
                        style,
                        Style::TelexToneEnd | Style::VniToneEnd | Style::TelexToneAfterNucleus
                    )
                {
                    continue;
                }
                let kk = case_keys(&k, case);
                let want = case_word(syl, case);
                for ctx in ["alone", "vi_sentence", "caps_run"] {
                    if ctx == "caps_run" && (case != 2 || is_vni(style)) {
                        continue;
                    }
                    let (typed, expect) = if ctx == "caps_run" {
                        (
                            format!("TIEENGS VIEEJT {kk} "),
                            format!("TIẾNG VIỆT {want} "),
                        )
                    } else if ctx == "alone" {
                        (format!("{kk} "), format!("{want} "))
                    } else if is_vni(style) {
                        (format!("to6i d9i {kk} "), format!("tôi đi {want} "))
                    } else {
                        (format!("tooi ddi {kk} "), format!("tôi đi {want} "))
                    };
                    let got = type_keys(method, &typed);
                    let e = stats.entry((style, ctx, case)).or_default();
                    e.0 += 1;
                    if got != expect {
                        let placement = tone_normal(&got) == tone_normal(&expect);
                        if placement {
                            e.2 += 1;
                        } else {
                            e.1 += 1;
                        }
                        let _ = writeln!(
                            logs.entry((style, ctx, case)).or_default(),
                            "{}\t{}\t{}\t{}",
                            if placement { "PLACE" } else { "FAIL" },
                            kk,
                            expect.trim_end(),
                            got.trim_end()
                        );
                    }
                }
            }
        }
    }
    // Backspace round trip: type the word, erase the last k keystrokes, type
    // them again - the screen must match typing the word straight through,
    // and every intermediate diff must keep the screen equal to the engine's
    // own view of it.
    let mut bs_total = 0usize;
    let mut bs_fail = 0usize;
    let mut bs_log = String::new();
    for syl in &syls {
        let chs = decompose(syl).unwrap();
        for style in ALL_STYLES {
            let Some(k) = keys(&chs, style) else { continue };
            let method = if is_vni(style) {
                InputMethod::Vni
            } else {
                InputMethod::Telex
            };
            let direct = type_keys(method, &k);
            let kc: Vec<char> = k.chars().collect();
            for back in 1..=kc.len() {
                bs_total += 1;
                let mut e = UltraFastViEngine::new();
                e.set_input_method(method);
                let mut screen = Vec::new();
                let mut ok = true;
                for &ch in &kc {
                    let (bs, s) = e.feed_diff(ch);
                    let s = s.to_string();
                    apply(&mut screen, bs, &s);
                }
                for _ in 0..back {
                    let (bs, s) = e.backspace_diff();
                    let s = s.to_string();
                    apply(&mut screen, bs, &s);
                    if screen.iter().collect::<String>() != e.current_output_diff().to_string() {
                        ok = false;
                    }
                }
                let mid: String = screen.iter().collect();
                let want_mid = type_keys(method, &kc[..kc.len() - back].iter().collect::<String>());
                for &ch in &kc[kc.len() - back..] {
                    let (bs, s) = e.feed_diff(ch);
                    let s = s.to_string();
                    apply(&mut screen, bs, &s);
                }
                let got: String = screen.iter().collect();
                if !ok || got != direct || mid != want_mid {
                    bs_fail += 1;
                    let _ = writeln!(
                        bs_log,
                        "{style:?}\t{k}\tback={back}\tmid={mid} (want {want_mid})\tgot={got} (want {direct})\tinvariant={ok}"
                    );
                }
            }
        }
    }
    println!("backspace round trip: total {bs_total} fail {bs_fail}");
    std::fs::write(format!("{out_dir}/backspace.tsv"), bs_log).unwrap();

    println!(
        "{:<24} {:<12} {:>4} {:>7} {:>6} {:>6}",
        "style", "ctx", "case", "total", "fail", "place"
    );
    for ((style, ctx, case), (t, f, p)) in &stats {
        println!(
            "{:<24} {:<12} {:>4} {:>7} {:>6} {:>6}",
            format!("{style:?}"),
            ctx,
            case,
            t,
            f,
            p
        );
    }
    for ((style, ctx, case), log) in &logs {
        let path = format!("{out_dir}/{style:?}_{ctx}_{case}.tsv");
        std::fs::write(&path, log).unwrap();
    }
    println!("failure logs in {out_dir}");
}
