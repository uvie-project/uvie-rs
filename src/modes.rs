pub const IS_VOWEL: u8 = 1 << 0;
pub const IS_MODIFIER: u8 = 1 << 1;
pub const IS_TONE_KEY: u8 = 1 << 2;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InputMethod {
    Telex,
    Vni,
    SimpleTelex,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ResolverKind {
    Telex,
    Vni,
}

pub struct Mode {
    pub classify: &'static [u8; 256],
    pub tone: &'static [u8; 256],
    pub w_target: &'static [bool; 256],
    pub resolver: ResolverKind,
    pub enable_w_bubbling: bool,
}

pub fn mode_for(method: InputMethod) -> &'static Mode {
    match method {
        InputMethod::Telex => &TELEX_MODE,
        InputMethod::Vni => &VNI_MODE,
        InputMethod::SimpleTelex => &SIMPLE_TELEX_MODE,
    }
}

const TELEX_MODE: Mode = Mode {
    classify: &CLASSIFY_TELEX,
    tone: &TONE_TELEX,
    w_target: &W_TARGET_TELEX,
    resolver: ResolverKind::Telex,
    enable_w_bubbling: true,
};

const VNI_MODE: Mode = Mode {
    classify: &CLASSIFY_VNI,
    tone: &TONE_VNI,
    w_target: &W_TARGET_VNI,
    resolver: ResolverKind::Vni,
    enable_w_bubbling: false,
};

/// Simple Telex mode: same tone keys and circumflex (aa/ee/oo/dd) as Telex,
/// but `w` is `vneHookAll` — it always applies horn/breve to ALL vowel
/// candidates, with no standalone `w→ư`, no double-w cancel, and no w-as-ư
/// at onset. This matches UniKey/macOS "Simple Telex" behavior.
const SIMPLE_TELEX_MODE: Mode = Mode {
    classify: &CLASSIFY_TELEX, // Same classification as Telex
    tone: &TONE_TELEX,         // Same tone keys as Telex
    w_target: &W_TARGET_TELEX, // Same w-target vowels (u, o, a)
    resolver: ResolverKind::Telex,
    enable_w_bubbling: true, // Same bubbling as Telex
};

/// Static-dispatch trait for monomorphized resolver calls.
/// TelexMode and VniMode are zero-sized types used as type parameters.
pub trait ModeTrait {
    const CLASSIFY: &'static [u8; 256];
    const TONE: &'static [u8; 256];
    const W_TARGET: &'static [bool; 256];
    const ENABLE_W_BUBBLING: bool;

    fn resolve(curr: u8, next: u8) -> (char, bool);
}

pub struct TelexMode;
impl ModeTrait for TelexMode {
    const CLASSIFY: &'static [u8; 256] = &CLASSIFY_TELEX;
    const TONE: &'static [u8; 256] = &TONE_TELEX;
    const W_TARGET: &'static [bool; 256] = &W_TARGET_TELEX;
    const ENABLE_W_BUBBLING: bool = true;

    #[inline(always)]
    fn resolve(curr: u8, next: u8) -> (char, bool) {
        resolve_telex(curr, next)
    }
}

pub struct VniMode;
impl ModeTrait for VniMode {
    const CLASSIFY: &'static [u8; 256] = &CLASSIFY_VNI;
    const TONE: &'static [u8; 256] = &TONE_VNI;
    const W_TARGET: &'static [bool; 256] = &W_TARGET_VNI;
    const ENABLE_W_BUBBLING: bool = false;

    #[inline(always)]
    fn resolve(curr: u8, next: u8) -> (char, bool) {
        resolve_vni(curr, next)
    }
}

pub const CLASSIFY_TELEX: [u8; 256] = {
    let mut t = [0u8; 256];
    t[b'a' as usize] = IS_VOWEL;
    t[b'e' as usize] = IS_VOWEL;
    t[b'o' as usize] = IS_VOWEL;
    t[b'u' as usize] = IS_VOWEL;
    t[b'i' as usize] = IS_VOWEL;
    t[b'y' as usize] = IS_VOWEL;

    t[b'w' as usize] = IS_MODIFIER;
    t[b'd' as usize] = IS_MODIFIER;

    t[b's' as usize] = IS_TONE_KEY;
    t[b'f' as usize] = IS_TONE_KEY;
    t[b'r' as usize] = IS_TONE_KEY;
    t[b'x' as usize] = IS_TONE_KEY;
    t[b'j' as usize] = IS_TONE_KEY;
    t[b'z' as usize] = IS_TONE_KEY;
    t
};

pub const CLASSIFY_VNI: [u8; 256] = {
    let mut t = [0u8; 256];
    t[b'a' as usize] = IS_VOWEL;
    t[b'e' as usize] = IS_VOWEL;
    t[b'o' as usize] = IS_VOWEL;
    t[b'u' as usize] = IS_VOWEL;
    t[b'i' as usize] = IS_VOWEL;
    t[b'y' as usize] = IS_VOWEL;

    t[b'0' as usize] = IS_TONE_KEY;
    t[b'1' as usize] = IS_TONE_KEY;
    t[b'2' as usize] = IS_TONE_KEY;
    t[b'3' as usize] = IS_TONE_KEY;
    t[b'4' as usize] = IS_TONE_KEY;
    t[b'5' as usize] = IS_TONE_KEY;

    // VNI modifiers: 6=circumflex, 7=horn(o/u), 8=breve(a), 9=đ
    t[b'6' as usize] = IS_MODIFIER;
    t[b'7' as usize] = IS_MODIFIER;
    t[b'8' as usize] = IS_MODIFIER;
    t[b'9' as usize] = IS_MODIFIER;

    // In VNI, 'd' is also a modifier (for 'd9' → đ).
    t[b'd' as usize] = IS_MODIFIER;
    t
};

pub const W_TARGET_TELEX: [bool; 256] = {
    let mut t = [false; 256];
    t[b'a' as usize] = true;
    t[b'o' as usize] = true;
    t[b'u' as usize] = true;
    t[b'd' as usize] = true;
    t
};

pub const W_TARGET_VNI: [bool; 256] = [false; 256];

pub const TONE_TELEX: [u8; 256] = {
    let mut t = [0u8; 256];
    t[b's' as usize] = 1;
    t[b'f' as usize] = 2;
    t[b'r' as usize] = 3;
    t[b'x' as usize] = 4;
    t[b'j' as usize] = 5;
    t[b'z' as usize] = 0;
    t
};

pub const TONE_VNI: [u8; 256] = {
    let mut t = [0u8; 256];
    t[b'0' as usize] = 0;
    t[b'1' as usize] = 1;
    t[b'2' as usize] = 2;
    t[b'3' as usize] = 3;
    t[b'4' as usize] = 4;
    t[b'5' as usize] = 5;
    t
};

#[inline(always)]
fn resolve_telex(curr: u8, next: u8) -> (char, bool) {
    match (curr, next) {
        (b'a', b'a') => ('â', true),
        (b'a', b'w') => ('ă', true),
        (b'e', b'e') => ('ê', true),
        (b'o', b'o') => ('ô', true),
        (b'o', b'w') => ('ơ', true),
        (b'u', b'w') => ('ư', true),
        (b'd', b'd') => ('đ', true),
        (b'w', _) => ('ư', false),
        _ => (curr as char, false),
    }
}

#[inline(always)]
fn resolve_vni(curr: u8, next: u8) -> (char, bool) {
    match (curr, next) {
        (b'a', b'6') => ('â', true),
        (b'a', b'8') => ('ă', true),
        (b'e', b'6') => ('ê', true),
        (b'o', b'6') => ('ô', true),
        (b'o', b'7') => ('ơ', true),
        (b'u', b'7') => ('ư', true),
        (b'd', b'9') => ('đ', true),
        _ => (curr as char, false),
    }
}
