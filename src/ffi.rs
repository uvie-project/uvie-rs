use std::ffi::{c_char, c_int};
use std::ptr;
use std::sync::{Mutex, MutexGuard};

use crate::diff::Diffable;
use crate::engine::UltraFastViEngine;
use crate::modes::InputMethod;

// ===================================================================
// Single opaque engine type - diff-based API
// ===================================================================

/// Opaque handle to the engine. C code only ever holds a pointer to this type;
/// the fields are intentionally not part of the C ABI.
///
/// All `feed`/`backspace`/`commit` functions use the diff API: they return
/// a backspace count and write the new suffix into a caller-provided buffer.
pub struct UvieEngine {
    inner: Mutex<UltraFastViEngine>,
}

impl UvieEngine {
    fn new() -> Self {
        Self {
            inner: Mutex::new(UltraFastViEngine::new()),
        }
    }
}

// ===================================================================
// Internal helpers
// ===================================================================

fn lock_engine<'a>(engine: *mut UvieEngine) -> Option<MutexGuard<'a, UltraFastViEngine>> {
    if engine.is_null() {
        return None;
    }
    let engine_ref = unsafe { &*engine };
    engine_ref.inner.lock().ok()
}

fn lock_engine_const<'a>(engine: *const UvieEngine) -> Option<MutexGuard<'a, UltraFastViEngine>> {
    if engine.is_null() {
        return None;
    }
    let engine_ref = unsafe { &*engine };
    engine_ref.inner.lock().ok()
}

fn utf8_prefix_len(bytes: &[u8], max_len: usize) -> usize {
    if bytes.len() <= max_len {
        return bytes.len();
    }
    let mut cut = 0usize;
    for (idx, ch) in std::str::from_utf8(bytes)
        .ok()
        .into_iter()
        .flat_map(|s| s.char_indices())
    {
        let next = idx + ch.len_utf8();
        if next > max_len {
            break;
        }
        cut = next;
    }
    cut
}

fn write_output(out: &str, out_buf: *mut c_char, out_len: usize) -> usize {
    if out_buf.is_null() || out_len == 0 {
        return 0;
    }
    let bytes = out.as_bytes();
    let max_write = out_len.saturating_sub(1);
    let write_len = utf8_prefix_len(bytes, max_write);
    if write_len > 0 {
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), out_buf as *mut u8, write_len);
            *out_buf.add(write_len) = 0;
        }
    } else {
        unsafe {
            *out_buf = 0;
        }
    }
    write_len
}

// ===================================================================
// Lifecycle
// ===================================================================

#[unsafe(no_mangle)]
pub extern "C" fn uvie_engine_new() -> *mut UvieEngine {
    Box::into_raw(Box::new(UvieEngine::new()))
}

#[unsafe(no_mangle)]
pub extern "C" fn uvie_engine_free(engine: *mut UvieEngine) {
    if !engine.is_null() {
        unsafe {
            drop(Box::from_raw(engine));
        }
    }
}

// ===================================================================
// Configuration
// ===================================================================

#[unsafe(no_mangle)]
pub extern "C" fn uvie_engine_set_input_method(engine: *mut UvieEngine, method: c_int) {
    let _ = std::panic::catch_unwind(|| {
        if let Some(mut e) = lock_engine(engine) {
            let m = match method {
                0 => InputMethod::Telex,
                1 => InputMethod::Vni,
                2 => InputMethod::SimpleTelex,
                _ => return,
            };
            e.set_input_method(m);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn uvie_engine_get_input_method(engine: *const UvieEngine) -> c_int {
    std::panic::catch_unwind(|| {
        let Some(e) = lock_engine_const(engine) else {
            return -1;
        };
        match e.input_method() {
            InputMethod::Telex => 0,
            InputMethod::Vni => 1,
            InputMethod::SimpleTelex => 2,
        }
    })
    .unwrap_or(-1)
}

#[unsafe(no_mangle)]
pub extern "C" fn uvie_engine_set_quick_start(engine: *mut UvieEngine, enabled: c_int) {
    let _ = std::panic::catch_unwind(|| {
        if let Some(mut e) = lock_engine(engine) {
            e.set_quick_start(enabled != 0);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn uvie_engine_set_quick_telex(engine: *mut UvieEngine, enabled: c_int) {
    let _ = std::panic::catch_unwind(|| {
        if let Some(mut e) = lock_engine(engine) {
            e.set_quick_telex(enabled != 0);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn uvie_engine_set_modern_orthography(engine: *mut UvieEngine, enabled: c_int) {
    let _ = std::panic::catch_unwind(|| {
        if let Some(mut e) = lock_engine(engine) {
            e.set_modern_orthography(enabled != 0);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn uvie_engine_set_relaxed_coda(engine: *mut UvieEngine, enabled: c_int) {
    let _ = std::panic::catch_unwind(|| {
        if let Some(mut e) = lock_engine(engine) {
            e.set_relaxed_coda(enabled != 0);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn uvie_engine_set_english_override(engine: *mut UvieEngine, enabled: c_int) {
    let _ = std::panic::catch_unwind(|| {
        if let Some(mut e) = lock_engine(engine) {
            e.set_english_override(enabled != 0);
        }
    });
}

// ===================================================================
// Diff-based keystroke API
// ===================================================================

/// Feed a single ASCII character.
/// Returns the number of backspaces the caller must send,
/// and writes the new output suffix into `out_buf`.
#[unsafe(no_mangle)]
pub extern "C" fn uvie_engine_feed(
    engine: *mut UvieEngine,
    ch: c_char,
    out_buf: *mut c_char,
    out_len: usize,
) -> usize {
    std::panic::catch_unwind(|| {
        if engine.is_null() || out_buf.is_null() || out_len == 0 {
            return 0;
        }
        let c = ch as u8 as char;
        let Some(mut e) = lock_engine(engine) else {
            return 0;
        };
        let (backspaces, suffix) = e.feed_diff(c);
        write_output(suffix, out_buf, out_len);
        backspaces
    })
    .unwrap_or(0)
}

/// Handle backspace.
/// Returns backspace count, writes new output suffix into `out_buf`.
#[unsafe(no_mangle)]
pub extern "C" fn uvie_engine_backspace(
    engine: *mut UvieEngine,
    out_buf: *mut c_char,
    out_len: usize,
) -> usize {
    std::panic::catch_unwind(|| {
        if engine.is_null() || out_buf.is_null() || out_len == 0 {
            return 0;
        }
        let Some(mut e) = lock_engine(engine) else {
            return 0;
        };
        let (backspaces, suffix) = e.backspace_diff();
        write_output(suffix, out_buf, out_len);
        backspaces
    })
    .unwrap_or(0)
}

/// Commit the current word (call on space / punctuation / break key).
/// Returns backspace count (usually 0), writes output into `out_buf`.
#[unsafe(no_mangle)]
pub extern "C" fn uvie_engine_commit(
    engine: *mut UvieEngine,
    out_buf: *mut c_char,
    out_len: usize,
) -> usize {
    std::panic::catch_unwind(|| {
        if engine.is_null() || out_buf.is_null() || out_len == 0 {
            return 0;
        }
        let Some(mut e) = lock_engine(engine) else {
            return 0;
        };
        let (backspaces, suffix) = e.commit_diff();
        write_output(suffix, out_buf, out_len);
        backspaces
    })
    .unwrap_or(0)
}

/// Re-enter a previously committed word with `ch` appended to its raw
/// keystrokes (LabanKey-style post-commit editing: the user arrows back
/// onto a committed word and types a tone/modifier key).
/// `caret_back` is the caret distance (screen chars) back to the end of the
/// newest committed word; it must exactly match a word-end boundary in the
/// engine's committed-word history (0 = newest word, + rendered_len + 1 per
/// older word) or land strictly inside a committed word (mid-word edit).
/// Returns backspaces + 1 when handled (suffix written into `out_buf`,
/// `out_fwd_del` set to the number of forward-delete chars the host must
/// send — 0 for word-end edits, the old word's tail length for mid-word
/// edits), 0 when there is no match or `ch` is a word boundary.
/// `out_fwd_del` may be NULL (forward-delete count discarded; callers that
/// cannot forward-delete should not rely on mid-word edits firing).
#[unsafe(no_mangle)]
pub extern "C" fn uvie_engine_edit_at(
    engine: *mut UvieEngine,
    caret_back: usize,
    ch: c_char,
    out_buf: *mut c_char,
    out_len: usize,
    out_fwd_del: *mut usize,
) -> usize {
    std::panic::catch_unwind(|| {
        if engine.is_null() || out_buf.is_null() || out_len == 0 {
            return 0;
        }
        let c = ch as u8 as char;
        let Some(mut e) = lock_engine(engine) else {
            return 0;
        };
        let Some((backspaces, fwd_del, suffix)) = e.edit_at_caret_diff(caret_back, c) else {
            return 0;
        };
        write_output(suffix, out_buf, out_len);
        if !out_fwd_del.is_null() {
            unsafe { *out_fwd_del = fwd_del };
        }
        backspaces + 1
    })
    .unwrap_or(0)
}

/// Reset all engine state (composing + committed + diff tracking).
#[unsafe(no_mangle)]
pub extern "C" fn uvie_engine_reset(engine: *mut UvieEngine) {
    let _ = std::panic::catch_unwind(|| {
        if let Some(mut e) = lock_engine(engine) {
            e.reset_diff();
        }
    });
}

// ===================================================================
// Introspection
// ===================================================================

#[unsafe(no_mangle)]
pub extern "C" fn uvie_engine_is_composing(engine: *const UvieEngine) -> c_int {
    std::panic::catch_unwind(|| {
        let Some(e) = lock_engine_const(engine) else {
            return 0;
        };
        if e.is_composing_diff() { 1 } else { 0 }
    })
    .unwrap_or(0)
}

/// Get the accumulated committed text (auto-committed syllables from V-C-V).
/// Returns byte count of written string (excluding null terminator).
#[unsafe(no_mangle)]
pub extern "C" fn uvie_engine_committed_text(
    engine: *const UvieEngine,
    out_buf: *mut c_char,
    out_len: usize,
) -> usize {
    std::panic::catch_unwind(|| {
        if engine.is_null() || out_buf.is_null() || out_len == 0 {
            return 0;
        }
        let Some(e) = lock_engine_const(engine) else {
            return 0;
        };
        write_output(e.committed_text_diff(), out_buf, out_len)
    })
    .unwrap_or(0)
}

/// Get the current full output (committed + composing).
/// Returns byte count of written string (excluding null terminator).
#[unsafe(no_mangle)]
pub extern "C" fn uvie_engine_current_output(
    engine: *const UvieEngine,
    out_buf: *mut c_char,
    out_len: usize,
) -> usize {
    std::panic::catch_unwind(|| {
        if engine.is_null() || out_buf.is_null() || out_len == 0 {
            return 0;
        }
        let Some(e) = lock_engine_const(engine) else {
            return 0;
        };
        // Use diff-aware output: in diff mode, the visible text is
        // diff_committed + prev_rendered, NOT the legacy committed + out_buf.
        let text = e.current_output_diff();
        write_output(&text, out_buf, out_len)
    })
    .unwrap_or(0)
}

/// Get the raw keystroke chars of the current word — the WHOLE word, across
/// V-C-V syllable splits, tone cancels and English dictionary commits.
/// Hosts use this for "escape restores the literal keystrokes" (the rendered
/// word is replaced by what the user actually typed).
/// Writes an EMPTY string when the word outgrew the 24-keystroke raw buffer
/// (its prefix was committed mid-word): the host must not restore it, since
/// the raw no longer covers the rendered text.
/// Returns byte count of written string (excluding null terminator).
#[unsafe(no_mangle)]
pub extern "C" fn uvie_engine_raw_chars(
    engine: *const UvieEngine,
    out_buf: *mut c_char,
    out_len: usize,
) -> usize {
    std::panic::catch_unwind(|| {
        if engine.is_null() || out_buf.is_null() || out_len == 0 {
            return 0;
        }
        let Some(e) = lock_engine_const(engine) else {
            return 0;
        };
        let s = e.word_raw_string();
        write_output(&s, out_buf, out_len)
    })
    .unwrap_or(0)
}
