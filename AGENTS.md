# AGENTS.md — uvie-rs Engine

## Build & Test Commands

```bash
rtk cargo build --release          # Build release
rtk cargo test --release           # Run all tests (277 tests)
rtk cargo bench --bench perf -- --warm-up-time 1 --measurement-time 3  # Benchmarks
```

### Benchmark suite (scenario-based, 2026-09)

`benches/perf.rs` benchmarks the engine's own APIs only — no `vi` crate
comparison. One criterion iteration = one natural typing unit; group names
match the scenario table: `compound_word/nghieengs` (→ nghiếng),
`random_keystroke_sequence/seeded_32`, `worst_case_deep_syllable/dduwowcj`
(→ được), `mixed_typing/viet_english` (dict-override words),
`rapid_backspace_burst/burst` (type + full backspace walk),
`english_passthrough/ghost`, `feed/legacy`. `Throughput::Elements` reports
keystrokes so per-keystroke cost is comparable across scenarios.


## Architecture

The engine has two APIs:
- **`feed()`** — legacy non-diff API (used by benchmarks as `type_seq`)
- **`feed_diff()`** — diff API (what the Swift app actually calls via FFI)
  - Returns `(backspaces, suffix)` — minimal edit instructions
  - Tracks `prev_rendered` (on-screen) vs `out_buf` (engine output)
  - V-C-V split: auto-commits first syllable when a vowel starts a new one
- **`edit_at_caret_diff(caret_back, ch)`** — post-commit word editing
  (LabanKey-style; FFI `uvie_engine_edit_at`)
  - The host tracks the caret distance (`caret_back`, screen chars) back to
    the end of the newest committed word. The engine walks its committed-word
    ring (cap 8, each entry = raw + rendered) accumulating
    `rendered_len + 1` boundary char per older word; the edit fires when
    `caret_back` EXACTLY matches a word-end boundary (0 = newest word) or
    lands strictly inside a word (mid-word edit). Unmatched carets (word
    start, double spaces, unseen caret jumps) return None — the host feeds
    the key normally.
  - Returns `(backspaces, forward_deletes, suffix)`. Word-end edits get
    `forward_deletes = 0` and `suffix` = the diffed tail; mid-word edits
    rewrite the whole word: `backspaces` = rendered chars left of the
    caret, `forward_deletes` = the old word's tail (chars right of the
    caret — the host must remove them first), and `suffix` = the full
    re-rendered word, posted after the deletions. The caret ends at the
    edited word's end.
  - On a match: the target word AND all newer entries are popped (their
    anchor geometry is stale once the target is re-entered); older entries
    survive and stay editable. The extended raw is re-rendered from scratch
    through the live pipeline (`reset_diff()` + `feed_diff` replay —
    rebuilding composing state, V-C-V splits, the English override and the
    snapshot stack exactly as fresh typing would), then diffs
    old-rendered → new render.
  - Mid-word placement (`edit_mid_word`): the entry's raw is replayed on a
    scratch engine to recover each V-C-V syllable boundary and rendered
    span. Tone/modifier keys apply at the END of the caret's syllable raw
    (UniKey semantics: 's' inside "viet" → "viết"); literal keys insert at
    the caret's exact position inside the word raw. Edits at the word's
    first raw key, keys that would only expand an already-resolved
    nucleus, and positions the replay can't place all decline.
  - After a successful edit the engine is COMPOSING the edited word, so
    backspace walks it back like normal composing and the next space
    re-records it. The host re-anchors its caret offset to 0.
  - Cost: zero on the per-keystroke hot path; the ring push is two small
    buffer copies once per space. The scratch replay only runs on edits.
- **`edit_newest_diff(ch)`** — post-commit word editing (LabanKey-style;
  added 2026-09, used by the Swift app via `uvie_engine_edit_newest`)
  - On every `commit_diff()` the word is recorded into a small ring in
    `DiffState` (`edit_history`, capacity 8: `raw` keystrokes + `rendered`
    on-screen text). Pushed AFTER the state clears — `DiffState::clear()`
    wipes the ring, so capture-then-push ordering matters.
  - `edit_newest_diff(ch)` pops the newest entry, appends `ch` to its raw,
    re-renders the extended word from scratch through the live pipeline
    (`reset_diff()` + `feed_diff` replay — this rebuilds composing state,
    V-C-V splits, the English override and the snapshot stack exactly as if
    the extended word had just been typed), then diffs old-rendered → new
    render and returns the `(backspaces, suffix)` instructions.
  - Returns `None` when the ring is empty, the engine is composing, `ch` is
    a word boundary, or the extended raw would overflow 24 chars — the host
    then feeds the key normally.
  - After a successful edit the engine is COMPOSING the edited word (the
    replay pushed per-keystroke snapshots), so backspace walks it back like
    normal composing and the next space re-records it.
  - Cost: zero on the per-keystroke hot path; the ring push is two small
    buffer copies once per space. The scratch replay only runs on edits.

## Performance Hotspots (measured 2026-07-03)

### Bottleneck: String (OutBuffer) clones on diff path
The `std` build uses `OutBuffer = String` (heap). Every `feed_diff` call
was cloning Strings 3-5 times. Fixed by using `mem::swap`/`mem::take`
and scratch buffers in `DiffState`.

### Bottleneck: `handle_telex_w` SylBuf clones
Was cloning the entire SylBuf (24 × Syl = 192 bytes) per candidate.
Fixed by snapshotting only the changed entry + restoring on failure.

### Bottleneck: `rerender_chars` creating new engine
Was allocating a fresh `UltraFastViEngine` on every V-C-V split.
Fixed with a `thread_local` scratch engine.

### Bottleneck: `diff_into` double char counting
Was calling `.chars().count()` twice. Fixed with single-pass counting.

## Performance Results (cumulative, 3 optimization rounds)

| Benchmark | Original | After Round 1 | After Round 2 | After Round 3 | Total |
|-----------|----------|---------------|---------------|---------------|-------|
| diff_short/phoos | 569 ns | 371 ns | 336 ns | 307 ns | **-46%** |
| diff_workaround/chuaw | 637 ns | 458 ns | 407 ns | 374 ns | **-41%** |
| diff_workaround/ngieengx | 1.18 µs | 814 ns | 733 ns | 674 ns | **-43%** |
| diff_sentences/long | 10.2 µs | 7.18 µs | 6.41 µs | 5.99 µs | **-41%** |
| diff_backspace/medium | 4.70 µs | 3.13 µs | 2.62 µs | 2.41 µs | **-49%** |
| diff_backspace/long | 4.54 µs | 3.11 µs | 2.63 µs | 2.41 µs | **-47%** |

### Accuracy Improvement (2025-01)
Data-driven testing with 30k Vietnamese Telex pairs:
- **Before**: 89.42% pass rate (3211 failures)
- **After**: 99.98% pass rate (6 failures — rare edge cases like `huow`→`huơ`)

Key fixes:
- Allow `e+tone+e` after consonant onsets (`befe`→`bề`, `biecse`→`biếc`)
- `cleanup_literal_tone_after_circumflex`: remove tone keys stuck in coda
  after circumflex is applied to an intermediate nucleus
- `cleanup_coda_tone_keys`: remove tone keys stuck in coda after `w` modifier
  transforms an intermediate nucleus into a valid one
- Silent `w` consume when all candidates have horn (`chuwongw`) or no valid
  candidate exists (`buwouw`)
- Circumflex validity check in `handle_vowel`: revert if result is invalid
  (`khoafo`→`khoào` instead of passthrough)
- Added missing triphthongs: `uây`, `oeo`, `uêu`
- `apply_coda_tone_rule`: added `oo` diphthong, fixed `n>=3` check
- `F_LITERAL` passthrough: check `is_valid_vietnamese()` before `F_LITERAL`
  (fixes `booongs`→`boóng` after triple-cancel)

Trade-off: ~40% regression on short benchmarks (~130ns/keystroke) due to
added `is_valid_vietnamese()` checks. Sentence/backspace benchmarks
remain faster than original baseline.

### Accuracy Improvement (2026-09): rhyme-level coda validation + /uə/ tone resolution
Follow-up to the hiatus fix, auditing the engine's accepted syllable space
against the hieuthi all-Vietnamese-syllables reference list (17,974):

1. **`rhyme_coda_compatible`** (`src/tables/nucleus.rs`) — rhyme-level
   (nucleus + coda) table replacing `nucleus_allows_coda` at both call sites
   (`is_legal_syllable`, `is_valid_vietnamese`). Encodes per-vowel coda
   restrictions: i/y take nh/ch not ng/c (`bings`→`bíng` fixed), ơ only
   m n p t (`sowrng`→`sởng` fixed), ư no p/nh/ch, ê keeps ng (ref has
   "êng"), diphthong sets per the standard rime table. Growth intermediates
   stay typeable: `c` accepted for e/o/u/i/y (c→ch, e→ê via oo, u→ư via w) —
   the data-driven scrambled tone-key encodings (`bicjh`→`bịch`,
   `becehj`→`bệch`) transit through those states. Relaxed-mode `g`/`h`
   shorthands bypass the check (teen code, "đawjh"→"đặh").
2. **/uə/ tone resolution** (`resolve_uo_rhyme_tone` in `src/tone_handler.rs`,
   called from `apply_coda_tone_rule`): plain [u,o] + consonant coda + tone →
   the second vowel takes the circumflex and carries the tone. Fixes the
   natural typing order `thuocs`→`thuốc`, `muons`→`muốn`, `nguonf`→`nguồn`,
   `thuocj`→`thuộc` (previously "thúoc"/"múon" — the tone landed on the u).
   The scrambled pair orders ("thuocos") already worked via the oo-merge.
   NOTE: the /uə/ rhyme is ALWAYS written uô when closed (thuộc = u+ộ, no
   horn — verified against the 22k list) and uơ (horn on the o only) when
   open (thuở). The horn form ("thưởng"/"xướng") requires the w key
   ("xuongsw"→"xướng" per the pairs data; "xuongso"→"xuống" without it).
3. **Triple-cancel guard** (`handle_vowel`): a toned circumflex vowel is a
   resolved rhyme (ố/ồ/ậ), not a user-typed "aa" — absorb the repeated
   vowel instead of cancelling ("thuocso" stays "thuốc").
4. Updated `mid_nucleus_tone_with_various_codas` vehicles: "thajach"→"thậch"
   and "iejech"→"iệch" used rhymes that don't exist (â+ch, iê+ch — iê takes
   c not ch); replaced with valid rhymes (â+ng, ê+ch "lệch").

### Accuracy Improvement (2026-09): open /uə/ "uơ" + deferred ư promotion
Fixes the last 6 telex_pairs failures ("huow"→"hươ" vs data "huơ") and 2
round-trip failures ("thuở", "đời thuở"):

1. **`['u','ơ']` nucleus** (tone_idx 1): the /uə/ OPEN rhyme is written "uơ"
   with the horn on the o only, plain u ("thuở", "huơ").
2. **Deferred ư promotion** (`try_apply_w_non_cancel`): u + w horns the o
   WITHOUT promoting the u. The promotion fires immediately only when the
   [u,ơ] is not a final open syllable — a coda or a trailing nucleus vowel
   follows ("buonw"→"bươn", "buoiw"→"ươi", "thuocws"→"thước"); a bare open
   "huow" stays "huơ". `promote_uo_horn_rhyme` (composing.rs) handles the
   coda-arrives-later order ("thuowcs"→"thước") and the triphthong
   completion ("huowi"→"hươi", "huowu"→"hươu").
3. **Circumflex→horn swap** (the 'o' branch): a resolved uô form + w → the
   /ɨə/ interpretation ("thuocsw"→"thước", "xuongsw"→"xướng",
   "dduongwf"→"đường").
4. **-ech resolution** (`resolve_ech_rhyme_tone`): plain [e] + coda "ch" +
   tone → the e takes the circumflex ("lechj"→"lệch" — the -ech rhyme is
   always written "êch" when toned; sắc/nặng only, 20+ words). [e]+ch is a
   growth intermediate (e→ê via the second e); [e]+ch with huyền has no
   words → passthrough.
5. Updated locked expectations to the open-form semantics: "huow"→"huơ",
   "huows"→"huớ", "uow"→"uơ", "uowj"→"uợ" (the ư promotion needs a
   following coda/vowel).

telex_pairs: 0 failures (was 6). round_trip: 158 (was 160).

Known remaining trade-offs: `thecs`→`théc` (the c→ch growth intermediate
accepts a toned state; needed for the scrambled pairs), `boong`→`bông` /
`xoong`→`xông` (oo→ô is fundamental Telex, same as UniKey), `dust`→`dút` in
the legacy feed() API (dict override handles it in the app's feed_diff path).

### Accuracy Improvement (2026-09): orthography-gated V-C-V + rime table cleanup
Two changes grounded in the standard Quốc Ngữ rime inventory (Vietnamese
orthography; "Các vần trong tiếng Việt"):

1. **Hiatus rule in the V-C-V split** (`src/diff/core.rs`). A syllable
   boundary inside a written token must fall on a consonant (the next
   syllable's onset) — vowel-to-vowel hiatus never occurs inside a
   Vietnamese token; any V-V sequence must be a single nucleus. The split
   now only fires when `find_split_point` returns an index before the new
   vowel (`split < len - 1`). Fixes `ressearch`→`rếarch` (the split
   resurrected the double-cancelled sắc as a committed `rế` syllable) and
   `theeo`→`thêo`. Consonant splits (`neebo`→`nêbo`, `toocaa`→`tôcâ`) are
   unchanged.

2. **Removed non-standard "(rare)" nuclei** (`src/tables/nucleus.rs`):
   `êo`, `ôu`, `ơu`, `ưo`, `io`, `ău`, `ăy` — zero attestations in the 22k
   word list and absent from the standard rime whitelist. These let English
   input render as fake Vietnamese (`theeo`→`thêo`, `keeo`→`kêo`,
   `tawuf`→`tầu`). Transient states (`uu`→ưu, `uo`→ươ precursor, `âo` for
   `naaos`→nấo, `oo` engine extension) are kept. Note: standard Telex maps
   `auw`→`âu` (UniKey-compatible); the engine currently consumes the `w`
   and passes through `au` — a possible future compatibility improvement.

Tests: `tests/orthography_tests.rs` (hiatus, passthrough, locked V-C-V).
Updated locked expectations in `bugfix_tests.rs` (`keeo`→passthrough),
`vcv_tests.rs` (`auw`→`au`), `word_boundary_tests.rs` (`dauw`→`dau`).

### Accuracy Improvement (2026-10): centering-diphthong post-coda tones + onset-exact V-C-V
UniKey/EVKey accept a tone key after the coda of a centering-rhyme word —
`tiens`→`tiến`, `viets`→`viết`, `tiengs`→`tiếng`, `tuans`→`tuấn`,
`hues`→`huế`, `kieux`→`kiễu`, `duocs`→`duốc` — but the engine previously
kept the key literal because the plain nucleus (`ie`, `ye`, `ue`, `ua`,
`eu`, and the triphthongs `ieu`, `yeu`, `uye`, `uoi`, `uay`, `ueu`) was
never legal in `is_valid_vietnamese`. Three pieces, mirroring the /uə/
(`resolve_uo_rhyme_tone`) and -ech precedents:

1. **Transient plain nuclei** (`src/tables/nucleus.rs`): the plain-vowel
   forms are legal nuclei, with `rhyme_coda_compatible` rows that only
   accept codas real words take (ie/ye→m n ng p t c; ue→m n nh p t ch c;
   ua→m n ng t c). An OPEN `ie`/`ye` nucleus stays INVALID in
   `is_valid_vietnamese` — iê/yê are always-closed written rhymes, so an
   open vowel pair can never take a tone key (`iej` stays literal;
   "tiens"→"tiến" is the only useful reading).
2. **`resolve_centering_rhyme_tone`** (`src/tone_handler.rs`, called from
   `apply_coda_tone_rule` before the modern-orthography early return):
   a toned plain centering nucleus resolves to the written circumflex
   form — ie/ye/ue/ua→ê at index 1, eu→ê at 0, ieu/yeu/uye/uoi/uay/ueu
   per the written accent — only when the nucleus has no horn/circumflex
   already (explicit `iee`/`woo` spellings are untouched).
3. **Onset-exact `find_split_point`** (`src/diff/utils.rs`): the V-C-V
   boundary is now the longest suffix of the consonant run that is a
   legal Vietnamese onset, not the run's first consonant. A tone key
   consumed mid-word stays inside syllable 1 (`neefbo`→`nềbo` was
   `nêfbo`); digraph onsets (`tr`, `ng`, `gh`) still travel together.
   Trade-off: sequences like `afghan` now render `àgha` mid-word (the
   'f' resolves as a tone on 'a' + `gh` onset) — an accepted loss since
   f/j/w/z are never Vietnamese consonants, and the dictionary override
   (regenerated to 15,416 words) restores them on completion.

### Accuracy Improvement (2026-08)
Onset↔nucleus distribution check (`onset_nucleus_compatible` in
`src/tables/onset.rs`). Vietnamese has a hard complementary distribution
for the palatal/velar series — `c`/`k`/`q` and `g`/`gh`/`ng`/`ngh`:
- `gh`/`ngh` only before `i`/`e`/`ê`
- `k` only before `i`/`e`/`ê`/`y`; `c` only before `a`/`ă`/`â`/`o`/`ô`/`ơ`/`u`/`ư`
- `g` before back vowels + `i` (standalone `gi`); `ng` before back vowels
- `q` only as the `qu` glide (length-1 `q` with a nucleus is invalid)

Without this, `is_valid_vietnamese` accepted e.g. `gh`+`o` as valid, so a
tone key inside an English word produced invalid Vietnamese such as
`ghost`→`ghót`. Now `gh`+`o` is rejected → `ghost` passes through. Wired
into both `is_valid_vietnamese` (`src/validation.rs`) and
`is_legal_syllable` (`src/tables/mod.rs`). No Vietnamese regression
(Telex pairs 99.98%, 22k round-trip 99.29% unchanged); English passthrough
improved 94.45%→94.63%.

Note: English words that produce *phonotactically valid* Vietnamese
syllables (e.g. `character`→`chẩcter`, `safari`→`sầri`, `reset`→`rết`)
cannot be passed through without a dictionary — the cross-tone
double-vowel circumflex path (`a`+tone+`a`→`â`) is required for valid
Vietnamese (`befe`→`bề`, `sầm` via `safam`). Restricting it regresses 851
Telex pairs. A dictionary-based check is the only clean fix.

### English Dictionary Override (2026-08, expanded 2026-10)
`src/tables/english.rs` — 15,416 common English words that produce garbled
Vietnamese+English hybrids (e.g. `character`→`chẩcter`, `safari`→`sầri`,
`good`→`gôd`, `book`→`bôk`). Sorted static `&[&str]` array with O(log n)
binary search — zero heap, zero dependencies, `no_std`-compatible.

The table is generated by `examples/gen_dict.rs`: a 100k-word English
corpus is typed through `feed_diff` with the override disabled; a word is
included iff its render differs from the raw word AND the render is not
valid Vietnamese — where "valid" means decomposable into real 22k words
(tone-aware) at consonant onsets, i.e. the same boundary the V-C-V split
produces. Candidates that would prefix-shadow a real Telex keystroke
sequence (from the 30k pairs file, e.g. "tojo" prefixing "tojot"→"tột")
are dropped unless the English word is common (top ~20k of the corpus) —
rare English loses to Vietnamese intent; common English wins, matching
the curated list's own precedent ("dust", "data"). Words ending in a
doubled tone key (ss/ff/rr/xx/jj) are dropped unless in the top ~10k:
the double key is the standard cancel sequence, and the 36 survivors
(`mass`, `pass`, `business`, …) keep working through the sticky
fallback documented below.

The feature can be toggled per engine (`set_english_override` /
`uvie_engine_set_english_override`, default on) so hosts can expose a
"restore English words" user preference.

The override fires **per-keystroke** (not just at word boundaries): as
soon as `word_raw` matches a dictionary word, the engine shows the raw
English word instead of the garbled Vietnamese transform. This means the
user sees "character" (not "chẩcter") while typing, before pressing space.

**Critical: state sync after override.** When the override fires, the
English word is committed to `diff_committed` and ALL composing state is
cleared (`raw_chars`, `key_log`, `buf`, `out_buf`, `prev_rendered`, etc.).
`word_raw` is preserved for future dict checks. This prevents ghost
characters when the user continues typing past a dict word (e.g. "good" →
"goodness"): without this, the V-C-V split would re-render the committed
portion from `raw_chars`, producing Vietnamese transforms ("gô") instead
of the English word ("good").

**Sticky English passthrough.** Once the override fires,
`diff.english_sticky` is set and the rest of the word passes through
raw — the tail is appended to `prev_rendered` and `word_raw` only
(`key_log` stays empty so backspace falls through to the committed-prefix
pop path). Without it the cleared suffix was re-transformed as fresh
Vietnamese, producing hybrids: `perm`+`ission`→"permision" (double-s
cancel ate a letter), `syst`+`ems`→"systém", `afric`+`ans`→"africán".
The flag clears on word boundary, commit, reset, and full-word erase
(typing Vietnamese resumes when the whole word is backspaced away).

**Sticky tail vs. the double-tone cancel.** A dictionary word ending in a
doubled tone key (`mass`, `pass`, `boss`, …) commits the literal raw, but
the pipeline's own raw is the *collapsed* form — the second key was the
standard Telex cancel (`mas`). Committing pins the rest of the word to
raw, which used to make the cancel untypeable for every such word
(`masster` stayed literal). Two pieces fix it generally:

- The override saves the pre-override pipeline state in
  `diff.english_fallback` (`EnglishFallback`: a `ComposingSnapshot` +
  `diff_committed` + the dictionary word's length) when the committed
  word ends with a doubled tone key (`ends_with_doubled_tone_key`) AND
  the pipeline actually collapsed a key (`raw_chars.len() <
  key_log.len()`, so a word like `class` whose `ss` stayed literal is
  not treated as a cancel).
- While that fallback is live, the word is re-opened as Vietnamese only
  when all three hold: the tail's first key is a consonant (a vowel tail
  is English — `mass`+`ive`, `less`+`er` — and re-interpreting it merges
  the vowel into the committed syllable, `lesser`→"lể"), the extended
  word is not a dictionary prefix, and the tail reads as Vietnamese
  (`tail_reads_as_vietnamese`: its own render carries a tone/modifier
  mark — `ter`→"tẻ", while `key`/`code`/`less`/`through` stay literal)
  with the re-rendered word staying plain letters
  (`word_renders_plain`: no Vietnamese diacritics — `restlessness` must
  not become "rétlesnes"). Then `fallback_to_vietnamese` restores the
  saved state, replays the tail and diffs the raw text on screen to the
  Vietnamese render: `masster`→"master", `misster`→"mister",
  `Masster`→"Master". Dictionary words and prefixes always stay raw
  (`mass`, `massive`, `passport`, `password`, `passing`, `passkey`,
  `passcode`, `passthrough`, `misspeak`).
- Backspacing below the dictionary word clears the saved fallback
  (`pop_word_raw`) — the snapshot no longer matches the typed prefix and
  restoring it would rewrite deleted text.

Known trade-off (16 of the 235k Webster word list, all obscure except the
two noted): `<cancel-word>+less/ness/...` compounds that are not in the
dictionary render collapsed — `lossless`→"losless", `massless`→"masless",
plus archaic forms (`bassness`, `mossless`, `passless`, `tariffless`,
`mosstrooping`, …). They are structurally identical to `masster` at every
keystroke, so only dictionary coverage can separate them. The dictionary
still applies at the word boundary for the words it holds, and Escape
restores the literal keystrokes.

The diff API tracks a lossless `word_raw: CharVec<24>` buffer in
`DiffState` that survives V-C-V splits and double-tone-cancel (unlike
the lossy `raw_chars`). At each keystroke, if `is_english_override
(&word_raw)` returns true, the engine recomputes the diff from the
pre-keystroke screen to the raw English word, then commits the word and
clears composing state. `word_raw_string()` (FFI
`uvie_engine_raw_chars`) exposes the WHOLE word's keystrokes — hosts
need that for "Escape restores the literal input": `raw_chars_string`
only holds the composing syllable, so after a V-C-V split or a
dictionary commit it would restore a fragment (`master` → "ter"). It
returns EMPTY when the word outgrew the 24-keystroke buffer
(`word_raw_overflowed`): the prefix was committed mid-word, so the raw
no longer covers the render and the host must cancel instead of
restoring.

**Excluded** from the dictionary:
- Words whose transform is a real Vietnamese word (from 22k word list):
  `chaos`→`cháo`, `most`→`mót`, `boots`→`bốt`, `deeds`→`đế`
- Words whose V-C-V split components are all valid Vietnamese words:
  `banana`→`bân`+`na`, or whose whole render is a real word (`user`→`uể`
  — 's' is sắc on u, 'e' joins the uê nucleus, 'r' is huyền)

Backspace in override state: after the override, the English word is in
`diff_committed` and `prev_rendered` is empty. Backspace pops from
`diff_committed` (and `word_raw` simultaneously) via the existing
`key_log.is_empty()` fallback path. No special override check needed.

Performance: the per-keystroke clones (`committed_before`, `prev_before`)
are gated by `word_raw.len() >= 4` (minimum dictionary word length),
so the common path (1-3 chars typed) has zero overhead. The dictionary
binary search is O(log 15670) ≈ 14 comparisons. Benchmarks show no
significant change on short benchmarks, -1.3% to -1.7% improvement on
sentence benchmarks, and -6.8% to -8.1% improvement on backspace
benchmarks (the old backspace override check is now dead code).

### Round 1: Eliminate heap allocations
- `diff_into`: single-pass char counting (was double `.chars().count()`)
- `feed_diff_core`: `mem::swap`/`mem::take` instead of `String::clone`
- `handle_telex_w`: snapshot only changed Syl entry (was cloning 192B SylBuf)
- `rerender_chars`: `thread_local` scratch engine (was allocating new engine)

### Round 2: Stack-only OutBuffer + partition cache
- `StackStr<N>`: `[u8; N]` UTF-8 buffer with `Deref<Target=str>` via
  `from_utf8_unchecked` — replaces `String`, zero heap
- `SylBuf.version()`: increments on every mutation, enables O(1) cache
  validity check for `partition_syllable()`
- `partition_syllable()` cache: avoids 5-10 redundant O(n) scans per keystroke

### Round 3: Branch prediction
- `feed_diff_core`: check cheap conditions first before touching scratch buffers
- Skip `scratch_display` build on common path (not optimistic)
- Optimistic display only built when `is_optimistic` is true (rare)

## Key Design Decisions

- `OutBuffer = String` for `std`, `heapless::String<128>` for `no_std`
- `SylBuf` is a fixed `[Syl; 24]` array (stack, no heap)
- `CharVec<24>` for raw keystroke tracking (stack, no heap)
- `partition_syllable()` is called multiple times per keystroke — it's
  O(n) where n ≤ 24, so it's fast but could be cached
- `is_valid_vietnamese()` calls `partition_syllable()` internally
- Traditional orthography: tone on first vowel for open syllables (hoá),
  second vowel for syllables with coda (hoạt, đoán)
- `apply_coda_tone_rule()` fixes tone placement at render time

## Coda Shorthands (teen code)

The engine accepts two kinds of final-consonant shorthands, both rendered
**verbatim** (the typed char is kept, not converted):

| Shorthand | Stands for | Mode | Tone rule |
|-----------|------------|------|-----------|
| `g`       | `ng`       | `--relaxed-coda` toggle | any tone (like ng) |
| `h`       | `nh`       | `--relaxed-coda` toggle | any tone (like nh) |
| `k`       | `c`        | **always active**       | sắc/nặng only (like c) |
| `nk`      | `nh`       | **always active**       | any tone (like nh) |

The `k` / `nk` shorthands are always on (no toggle) to support common
teen code and province spellings: "đắk", "Đăk Lak", "đỉnk" (= đỉnh).
Implementation lives in `src/tables/coda.rs` (`is_legal_coda`,
`tone_allowed_for_coda`) — validation only, no rendering conversion.

## Benchmark Cases

- `diff_short` — simple words exercising modifier/tone paths
- `diff_workaround` — words with many if/else branches (V-C-V splits,
  double-vowel circumflex, w modifier with multiple candidates,
  mid-nucleus tone, tone + coda + reapply)
- `diff_sentences` — long sentences with word boundaries
- `diff_backspace` — type word then backspace all the way back
- `compare_telex` — uvie vs vi-rs crate comparison
