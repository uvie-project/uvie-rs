//! Positive Vietnamese syllable pattern tables.
//!
//! Positive onset / nucleus / coda tables for Vietnamese syllable validation.
//!
//! # Validation strategy
//!
//! A word is Vietnamese iff:
//!   `is_legal_onset(onset) && nucleus_tone_target(nucleus).is_some() && is_legal_coda(coda)`
//!
//! Anything that does not match falls through as **literal passthrough**. This
//! automatically handles English words without any blacklist.
//!
//! # Tone-target index
//!
//! `nucleus_tone_target` returns `Some(i)` where `i` is the 0-based offset
//! within the nucleus slice that should receive the tone diacritic (modern
//! orthography). This replaces the 60-line `apply_tone_in_place` heuristic.
//!
//! # Sources
//!
//! - Vietnamese orthography standard (onset/coda/nucleus constraints).
//! - Cross-referenced against `src/tests.rs` for tone-placement tests.

mod coda;
mod english;
mod glide;
mod nucleus;
mod onset;

pub use coda::{is_legal_coda, tone_allowed_for_coda};
pub use english::{is_english_override, is_english_prefix};
pub use glide::{is_vowel_base, onset_is_gi, onset_is_qu};
pub use nucleus::{
    is_legal_nucleus, nucleus_allows_coda, nucleus_tone_target, rhyme_coda_compatible,
};
pub use onset::{is_legal_onset, is_onset_prefix, onset_nucleus_compatible};

/// Validates a complete syllable given its three components expressed as
/// resolved output characters.
///
/// - `onset_raw`:   raw base bytes of the onset (e.g. `b"th"`, `b"ngh"`).
/// - `nucleus_out`: resolved chars of the nucleus (e.g. `['ê']`, `['o','a']`).
/// - `coda_raw`:    raw base bytes of the coda (e.g. `b"ng"`, `b"t"`).
///
/// Returns `true` if all three components are legal and the tone is compatible
/// with the coda.
pub fn is_legal_syllable(
    onset_raw: &[u8],
    nucleus_out: &[char],
    coda_raw: &[u8],
    tone: u8,
    relaxed: bool,
) -> bool {
    is_legal_onset(onset_raw)
        && is_legal_nucleus(nucleus_out)
        && onset_nucleus_compatible(onset_raw, nucleus_out[0])
        && is_legal_coda(coda_raw, relaxed)
        && tone_allowed_for_coda(coda_raw, tone, relaxed)
        // Closing diphthongs and triphthongs cannot take consonant codas.
        // Only centering diphthongs and monophthongs allow codas.
        && (coda_raw.is_empty() || rhyme_coda_compatible(nucleus_out, coda_raw, relaxed))
}
