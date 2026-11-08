# uvie-rs

Ultra-fast Vietnamese input method engine (Telex, VNI, Simple Telex) written in Rust.

A zero-dependency library with **no heap allocation in the hot path**. Designed for sub-microsecond latency per keystroke through incremental state updates, positive syllable validation, and 100% stack-allocated buffers.

**macOS implementation**: See [UVieMac](https://github.com/uvie-project/uvie-mac) for the macOS menu bar app that uses this engine.

## Features

- **Telex, VNI & Simple Telex**: Full support for the popular input methods — including Simple Telex, where `w` always applies horn/breve to all vowel candidates (UniKey-compatible `vneHookAll` behavior).
- **Modern orthography**: Optional tone placement per new standard (e.g. `hoas` → `hoá`).
- **Relaxed coda mode**: Optional lenient coda validation for flexible typing.
- **Post-coda tone keys on centering diphthongs** (UniKey-compatible): `viets` → `viết`, `tiens` → `tiến`, `hues` → `huế`, `kieux` → `kiễu` — the tone key applies after the coda and the render resolves to the written circumflex form.
- **Committed-word editing** (LabanKey-style): `edit_at` re-enters any of the last 8 committed words — at the word-end boundary or a caret strictly inside the word — and returns `(backspaces, forward_deletes, suffix)` so the host can re-render it in place.
- **English override dictionary**: a ~15k-word table keeps English words literal (`permission`, `system`), with sticky passthrough so the rest of an overridden word stays raw. Words that would shadow real Telex sequences (e.g. `tojo` before `tojot` → `tột`) are filtered out.
- **Per-character state**: Each keystroke gets its own state entry; transforms are bit-flips, not multi-pass reordering.
- **Validate raw keystrokes**: Checks raw ASCII sequence against positive syllable tables before any transform. English passthrough is automatic.
- **Diff-based API**: Returns `(backspace_count, suffix_to_type)` per keystroke for minimal screen updates.
- **O(1) backspace**: Snapshot stack mechanism eliminates O(n²) rebuild on backspace.
- **No heap in hot path**: All buffer types (`StackStr`, `CharVec`, `SylBuf`) are stack-allocated — zero allocation per keystroke.
- **Zero dependencies**: No `heapless`, no external crates — pure Rust.

## Architecture

The engine uses an incremental, stateful model with per-character state buffers. See [`src/ARCHITECT.md`](src/ARCHITECT.md) for detailed design rationale, state model, and per-keystroke flow.

## Usage

```rust
use uvie::{UltraFastViEngine, InputMethod};

let mut engine = UltraFastViEngine::new();
engine.set_input_method(InputMethod::Telex);

// Feed keystrokes
let result = engine.feed('t');  // "t"
let result = engine.feed('i');  // "ti"
let result = engine.feed('e');  // "tie"
let result = engine.feed('s');  // "tiế"

// Commit word (e.g. on space)
engine.commit();
```

## FFI / C API

The library compiles to `staticlib` and `cdylib`. The C API provides:

- `uvie_engine_new()` / `uvie_engine_free()` / `uvie_engine_reset()`
- `uvie_engine_feed(engine, ch, out_buf, out_len)` — returns the backspace count; `out_buf` receives the suffix to type
- `uvie_engine_backspace(engine, out_buf, out_len)` / `uvie_engine_commit(...)`
- `uvie_engine_edit_at(engine, caret_back, ch, out_buf, out_len, out_fwd_del)` — committed-word edit; `out_fwd_del` reports the forward-deletes a mid-word edit needs
- `uvie_engine_is_composing(engine)` — engine state query
- `uvie_engine_raw_chars(engine, out_buf, out_len)` / `uvie_engine_current_output(...)` / `uvie_engine_committed_text(...)` — raw typed keys vs rendered form (Escape-restore)
- `uvie_engine_set_input_method(engine, method)` + configuration toggles (`uvie_engine_set_modern_orthography`, `uvie_engine_set_english_override`, `uvie_engine_set_quick_telex`, `uvie_engine_set_quick_start`, `uvie_engine_set_relaxed_coda`)

See [`src/ffi.rs`](src/ffi.rs) for the full C API.

## Building

```bash
# Build release library
cargo build --release

# Run benchmarks
cargo bench

# Run tests
cargo test
```

The crate ships `cdylib`/`staticlib` artifacts, which require `std`; the
engine internals stay free of heap allocation regardless.

## Prebuilt releases

GitHub releases provide ready-to-link static libraries:

| Platform | Asset | Contents |
| -------- | ----- | -------- |
| macOS universal | `uvie-macos-universal.tar.gz` | `libuvie.a` (arm64 + x86_64), `uvie.h` |
| Linux x86_64 | `uvie-linux-x86_64.tar.gz` | `libuvie.a`, `uvie.h` |
| Windows x86_64 | `uvie-windows-x86_64.zip` | `uvie.lib`, `uvie.h` |

See the [release workflow](.github/workflows/release.yml) for build details.

## Benchmark

Scenario-based criterion suite (`cargo bench`), Apple Silicon. One operation
= one natural typing unit (a word, a sentence, or a type+delete cycle), and
criterion's `Throughput::Elements` reports the keystroke count, so
per-keystroke cost is comparable across scenarios and across releases.

| Scenario | Input | Time/op | Keys |
| -------- | ----- | -------: | ---: |
| Compound Word | `nghieengs` → nghiếng | ~1.05 µs | 9 |
| Random Keystroke Sequence | 32 seeded-random letters | ~4.03 µs | 33 |
| Worst-case Deep Syllable | `dduwowcj` → được | ~922 ns | 9 |
| Mixed Typing (Viet + English) | 107-char sentence | ~11.3 µs | 107 |
| Rapid Backspace Burst | type được + full backspace walk | ~1.09 µs | 16 |
| English Passthrough | `ghost` (phonotactic rejection) | ~347 ns | 6 |
| Feed Benchmark | legacy non-diff `feed()` API | ~621 ns | 9 |

All diff scenarios go through `feed_diff` — the API the Swift app drives
over FFI. The legacy `feed()` case is kept for cross-version comparison.

## License

MIT OR Apache-2.0
