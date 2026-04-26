# Profiling guide

How to take a profile of `aozora`, what the existing probes tell you,
and the case studies that produced them.

This file consolidates the methodology developed across N1–N7 (lex
optimisation), R1 (renderer), and T1 (tokenizer SIMD investigation).
Each case study links to the commit where the work landed.

---

## Quick start

The two commands you'll use 95 % of the time:

```bash
# Sample-profile a single corpus document. Output: /tmp/aozora-doc-<basename>.json.gz
AOZORA_CORPUS_ROOT=/path/to/corpus \
  just samply-doc 001529/files/50685_ruby_67979/50685_ruby_67979.txt

# Sample-profile the parser hot path across the full corpus
# (5 parse passes after the one-time load by default).
AOZORA_CORPUS_ROOT=/path/to/corpus just samply-corpus

# Sample-profile the HTML render hot path. 5 renders per doc by default.
AOZORA_CORPUS_ROOT=/path/to/corpus just samply-render

# Open any of the resulting JSON traces in the Firefox-Profiler UI.
samply load /tmp/aozora-corpus-<timestamp>.json.gz
```

All three are thin wrappers over `aozora-xtask`'s `samply` subcommand.
Source: `crates/aozora-xtask/src/main.rs`. They run on the host
(not Docker — `perf_event_open(2)` doesn't survive the container
seccomp profile) and rebuild with `--profile=bench` so debug info is
preserved.

---

## Probes (per-band corpus measurement)

Each probe under `crates/aozora-bench/examples/` reports a different
slice of the workload. All read `AOZORA_CORPUS_ROOT`; most accept
`AOZORA_PROFILE_LIMIT=N` to cap the sweep.

| Probe | Question it answers | Output shape |
|---|---|---|
| `throughput_by_class` | Per-band MB/s for `lex_into_arena` | 4-band table + p50/p90/p99/max + ns/byte |
| `phase_breakdown` | Per-phase ms for sanitize / tokenize / pair / classify | per-doc latencies + top-5 worst classify / sanitize |
| `latency_histogram` | Log-bucketed latency distribution per phase | bar histogram, 10 buckets, 1 µs … 1 s |
| `pathological_probe` | Single-doc 100-iter avg per phase | tight per-call numbers; takes `AOZORA_PROBE_DOC` for any corpus path |
| `phase0_breakdown` | Per-sub-pass cost inside Phase 0 sanitize | bom_strip / crlf / rule_isolate / accent / pua_scan |
| `phase0_impact` | Does Phase 0 sub-pass firing change Phase 1 cost? | bucketed by which sub-passes fired |
| `phase3_subsystems` | Per-recogniser ms inside classify | requires `--features instrument` (`aozora-lexer/phase3-instrument`) |
| `diagnostic_distribution` | What fraction of docs emit diagnostics? | histogram by diag count; latency-by-diag-bucket |
| `allocator_pressure` | Arena bytes / source byte ratio + intern dedup | per-doc histograms |
| `fused_vs_materialized` | Does the I-2 deforestation actually win? | per-band gap % between fused (lex_into_arena) and materialized (per-phase collect) |
| `intern_dedup_ratio` | How well does the interner dedup short strings? | corpus-aggregate (cache + table) / calls |
| `render_hot_path` | Per-band MB/s for HTML render | 4-band MB/s + render/parse ratio + out/in size ratio |

Each probe can be invoked directly:

```bash
AOZORA_CORPUS_ROOT=… cargo run --release --example <name> -p aozora-bench
```

For `phase3_subsystems`, build with the instrumentation feature:

```bash
AOZORA_CORPUS_ROOT=… cargo run --release --features instrument \
  --example phase3_subsystems -p aozora-bench
```

---

## Common pitfalls

These caught us at least once during the N-series and R1 work:

### 1. `cargo run --release` strips debug info

`cargo run --release` builds with `[profile.release]`, which has
`debug = 0` + `strip = "debuginfo"`. Samply will record samples but
the addresses won't symbolicate cleanly. Use `--profile=bench`
instead — the workspace `[profile.bench]` inherits from release but
sets `debug = 1` + `strip = "none"`. The xtask wrappers do this
automatically.

Symptom if you forget: samply output shows function addresses (e.g.
`0x8fb61`) instead of names. `nm` / `objdump --syms` returns "no
symbols". Re-run via the xtask.

### 2. `perf_event_paranoid` must be ≤ 1

Samply uses `perf_event_open(2)` for kernel sampling. Linux's
default is `2` (block all unprivileged perf access). Set once per
boot:

```bash
echo 1 | sudo tee /proc/sys/kernel/perf_event_paranoid
```

The xtask wrappers refuse to launch and print the fix-up command if
the value is too high.

### 3. Corpus load dominates a samply trace

`throughput_by_class` and `render_hot_path` spend most wall time in
Shift-JIS decode + filesystem I/O during the one-time corpus load.
A single-pass samply trace puts `__memmove_avx_unaligned` and
`encoding_rs::ShiftJisDecoder` at the top — *not* the parser.

Fix: set `AOZORA_PROFILE_REPEAT=K` (or pass `K` to
`just samply-corpus`) so the parse pass runs `K` times after the
load. The xtask defaults to 5; raise to 10+ for very small corpora.
The probe report still shows per-doc numbers from the final pass.

### 4. `cargo clippy --workspace --all-targets` is NOT what CI runs

CI (and the lefthook pre-commit hook) runs:

```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

`--all-features` enables `phase3-instrument` on `aozora-lexer` and
`extension-module` on `aozora-py`, which exposes additional code
paths to clippy. Local checks without `--all-features` will silently
miss real warnings. Run the full form, or just let the pre-commit
hook do it for you.

### 5. The bench profile binary is shared with `cargo run --release`

Both write to `target/release/examples/<name>`. If you build with
`--profile=bench`, then run `cargo run --release --example <name>`,
the second invocation **overwrites** the first with a stripped
binary. The xtask defends against this by always rebuilding before
recording.

### 6. Subsystem instrumentation distorts the numbers it reports

`phase3-instrument` wraps every recogniser entry in a
`SubsystemGuard` that calls `Instant::now()` on construction +
drop. For the dominant inner-loop recognisers this adds enough
overhead that the **report's own timing is significantly skewed**.
Use the instrumentation to compare *relative* costs between
subsystems, not as an absolute number. For absolute numbers, run
`phase_breakdown` (no instrumentation).

---

## Investigation case studies

A non-exhaustive index of past optimisation work, what samply
showed, and the conclusion. Each entry links to the commit and any
ADR.

### N1: Phase 3 instrumentation harness

Added the `phase3-instrument` feature (17 sub-system guards) +
`phase3_subsystems` example so subsequent N-work could see *which*
recogniser dominated on a given doc. No perf change of its own;
this was the diagnostic substrate for everything downstream.

Commits: `9061d38`–`7f5c1dc` (G.5 + N1).

### N2: Outlier doc 49178 — `pending_outputs` `O(N²)` memmove

Doc 49178 (corpus 232 KB) classified in **47 ms** — 130× per-byte
slower than baseline. samply attributed 96 % of CPU to glibc's
`__memmove_avx_unaligned`. Root cause: `ClassifyStream::pending_outputs`
was a `SmallVec<[ClassifiedSpan; 4]>` drained one-per-`next()` via
`SmallVec::remove(0)` (an `O(N)` backshift). The
`replay_unrecognised_body` path can push 16 k+ spans in a single
burst (top-level Quote pairs with no recogniser), turning per-yield
front-pop into a quadratic memmove storm.

Fix: swap to `VecDeque<ClassifiedSpan>` (ring buffer, `O(1)`
`pop_front`).

Result: doc 49178 classify **47 ms → 0.69 ms** (68× faster). Corpus
throughput **180 MB/s → 254.6 MB/s** (+41 %). The 50 KB–500 KB band
max latency: **~50 ms → 4.69 ms**.

Commit: `d82ff57`.

### N3: Top-level Quote / Tortoise stream-through

After N2 doc 49178 was fast, but the classifier still buffered
~24 k events per top-level Quote pair, ran the recogniser-decline
path, then replayed every event back. Pure overhead for an
unrecognisable pair.

Fix: when `handle_top_level` sees a top-level `PairOpen{Quote |
Tortoise}` (no recogniser candidate), enter `StreamingFrame` mode
instead of `open_frame`. Body events flow directly through
`handle_stream_event` (mirrors `replay = true` semantics) with a
nested-open depth counter. Outer close exits streaming.

Result: corpus mean classify **86.86 µs → 81.26 µs** (-6.4 %),
medium/large band throughput **+5.7 % to +9.5 %**. Doc 49178 itself
moved within noise (N2 had already collapsed its quadratic work);
the win is broader corpus.

Commit: `8aa4873`.

### N4: Interner short-string fast paths — investigated and rejected

Hypothesis: `Interner::intern` was at ~17 % of doc 50685's classify
time, and corpus dedup ratio was only 27.5 % (so 70 % of intern
calls were first-time inserts). A `≤16-byte chunk-load hash` and a
`2-slot LRU inline cache` were both implemented and benchmarked.

Result on the chunk-load hash: with no avalanche finalizer,
`average_probe_length_stays_low_at_typical_load` failed (avg probe
~42 vs threshold 2). With the xxHash-style avalanche, the test
passed but the two extra multiplications cost more than the
per-byte loop saved. Corpus throughput moved within noise (-4 % to
+2 % depending on band).

Result on the 2-slot cache: corpus dedup ratio stayed identical
(p50 0.275, mean 0.308) — the alternating-pair pattern the design
assumed is rare in real corpora.

Both reverted. Doc-comment in `crates/aozora-syntax/src/borrowed/intern.rs`
captures the negative result so the next person reaching for the
same idea sees the prior data without re-deriving it.

Commit: `0fd08cc`.

### N5: `throughput_by_class` parser-only wall split

The probe used to report a single "wall" number that conflated
corpus load (Shift-JIS decode + bucketing) with the parse pass it
actually measured. Samply traces of the binary then showed the
load syscalls dominating even though the probe's per-doc latencies
were measuring only the parse.

Fix: split `wall` into `load wall` and `parse wall`. Add
`AOZORA_PROFILE_REPEAT=K` so a parser-bound trace can be requested
explicitly.

Commit: `7f5c1dc` (part of G.5 / N5).

### N6: Pre-size the document arena

`Document::new` used `Arena::new()` (default `bumpalo` chunk size).
The `allocator_pressure` probe revealed `arena_bytes / source_byte`
p50 = 3.4×, p99 = 8.25×, max 15.4× across the corpus. Pre-sizing
the arena to `source.len() × 4` (factor covers p50 + margin)
eliminates early chunk-grow churn on large docs.

Commit: `41f7ab2`.

### N7: Replace shell `samply-{doc,corpus}.sh` with `aozora-xtask`

Two scripts in `scripts/` had drifted into bash-idiom territory.
Rewritten as a Rust workspace crate (`aozora-xtask`) with `clap`
subcommands. The Justfile gains `just samply-doc` /
`samply-corpus` / `samply-render` targets that delegate to it.
Hosts: the binary runs on the host (not Docker — see pitfall
§ 2 above).

Commit: `0f10fcf`.

### R1: Renderer byte-level memchr scan

Samply on `render_hot_path` showed `aozora_render::html::render_into`
spending the bulk of its time in `core::str::iter::MatchIndices::next`
walking every codepoint via the `is_structural_char` predicate
(`Chars::next` + `next_code_point`). For a 200 KB doc that's
~67 k char-iter calls, almost all returning false.

Fix: every PUA sentinel (E001..E004) shares the 2-byte UTF-8
prefix `0xEE 0x80`. The other structural character is `\n`. One
`memchr2(0xEE, 0x0A)` finds candidates at memory-bandwidth via
SIMD; each is validated with two byte loads to confirm the full
sentinel codepoint. PUA collisions (recorded by Phase 0
diagnostics but not deleted) flow through as plain via the cursor
advance. Same byte-scan optimisation applied to
`escape_text_chunk`'s HTML-unsafe lookup (rare in Japanese prose →
two `memchr` passes fast-skip).

Same optimisation also applied to `serialize::serialize_into`.

Result: render throughput by band:

  | Band | Before MB/s | After MB/s | Δ |
  |---|---|---|---|
  | <50 KB | 671 | 1066 | +59 % |
  | 50 KB–500 KB | 736 | 1280 | +74 % |
  | 500 KB–2 MB | 625 | 992 | +59 % |
  | >2 MB | 419 | 496 | +18 % |

Render is now ~5–6× faster than parse. The post-fix samply trace
no longer surfaces any `aozora_render::*` frame in the top-25 hot
addresses; render shrunk to a thin wrapper around memchr's SIMD
scan.

Commit: `f8fc0ea`.

### T1: Phase 1 SIMD tokenizer — investigated and reverted

Hypothesis: wire the existing `aozora_lex::tokenize_with_scan`
(SIMD-driven via `aozora-scan`) into the production
`Pipeline::tokenize` (currently calls the legacy char-by-char
walker). The aozora-scan crate already shipped with `ScalarScanner`
(memchr3 over `{0xE2, 0xE3, 0xEF}`) + `Avx2Scanner` (handwritten
AVX2). Estimated 5–6× win.

Result on doc 49178 (232 KB Japanese-heavy):
- Legacy walker: **0.41 ms** tokenize (570 MB/s)
- SIMD scan rewrite: **1.50 ms** tokenize (155 MB/s) — **3.7× SLOWER**

Root cause: `0xE3` is the leading UTF-8 byte of *every* Japanese
codepoint (hiragana, katakana, common kanji). `memchr3(0xE2, 0xE3,
0xEF)` returns ~every third byte of Japanese-heavy source as a
candidate, and the per-candidate PHF lookup costs about the same
as the legacy walker's UTF-8 decode + 11-arm `match`. The
"trigger density < 0.5 %" assumption that motivated aozora-scan's
design holds for *triggers*, but candidate density is set by
*leading-byte* density which on Aozora is ~33 %.

Reverted. ADR-0013 documents the architectural decision; the
`phase1_events.rs` module docstring records the perf data inline.
Three follow-up strategies enumerated in the ADR:

1. simdjson-style structural bitmap with full 3-byte compare per
   trigger (no false positives, but ~33 ops per chunk).
2. DFA over UTF-8 bytes recognising every trigger.
3. **Middle-byte filter**: scan for the 2nd UTF-8 byte (which is
   `0x80` for 7 of the 11 triggers and `0xBC` / `0xBD` for the
   other 4) — these are ~1 % of Japanese bytes vs `0xE3`'s ~33 %.

The `Avx2Scanner::byte_identical_to_scalar` proptest was extended
in this commit to cover up to 16 KiB inputs (was 300 codepoints).
A `best_scanner_name() -> "avx2" | "scalar"` diagnostic was added
to `aozora-scan` so callers can confirm at a glance which backend
would be selected without needing samply.

Commit: `57e0eef`. ADR: `docs/adr/0013-aozora-scan-leading-byte-strategy-loses-on-japanese.md`.

---

## Workflow recipes

### "I changed something, did I regress?"

```bash
# Microbench the per-band tokenizer throughput.
cargo bench -p aozora-lex --bench tokenize_compare

# Macrobench the full pipeline end-to-end.
AOZORA_CORPUS_ROOT=… cargo run --release --example throughput_by_class -p aozora-bench
AOZORA_CORPUS_ROOT=… cargo run --release --example render_hot_path -p aozora-bench

# Check the worst doc didn't regress.
AOZORA_CORPUS_ROOT=… AOZORA_PROBE_DOC=000286/files/49178_ruby_58807/49178_ruby_58807.txt \
  cargo run --release --example pathological_probe -p aozora-bench
```

### "Where is `lex_into_arena` spending its time?"

```bash
# Macroscopic per-phase split.
AOZORA_CORPUS_ROOT=… cargo run --release --example phase_breakdown -p aozora-bench

# Latency tail shape.
AOZORA_CORPUS_ROOT=… cargo run --release --example latency_histogram -p aozora-bench

# Microscopic: which classify recogniser dominates a specific doc?
AOZORA_CORPUS_ROOT=… AOZORA_PROBE_DOC=… \
  cargo run --release --features instrument --example pathological_probe -p aozora-bench
```

### "Take a samply trace I can open in Firefox-Profiler"

```bash
# Single doc.
AOZORA_CORPUS_ROOT=… just samply-doc 001529/files/50685_ruby_67979/50685_ruby_67979.txt
samply load /tmp/aozora-doc-50685_ruby_67979.json.gz

# Full corpus, parse-bound.
AOZORA_CORPUS_ROOT=… just samply-corpus 5
# /tmp/aozora-corpus-<timestamp>.json.gz

# Full corpus, render-bound.
AOZORA_CORPUS_ROOT=… just samply-render 5
# /tmp/aozora-render-<timestamp>.json.gz
```

### "Confirm AVX2 is actually firing"

```rust
// In any binary or test:
println!("{}", aozora_scan::best_scanner_name());
// Prints "avx2" or "scalar" — pure inspection, no SIMD work.
```

Or under samply: look for `aozora_scan::backends::avx2::scan_offsets_avx2`
in the trace's call tree. If the trace shows
`memchr::arch::x86_64::avx2::*` instead, you're on the scalar
fallback (which uses memchr's own SIMD dispatch internally — still
SIMD, just not aozora-scan's).

---

## Where things live

| Path | What |
|---|---|
| `crates/aozora-bench/examples/*.rs` | the 12 probes |
| `crates/aozora-bench/src/lib.rs` | `corpus_size_bands` + `log_histogram_ns` + `render_bar_row` (probe helpers) |
| `crates/aozora-xtask/src/main.rs` | `xtask samply <doc | corpus | render>` |
| `crates/aozora-lexer/src/instrumentation.rs` | the 17 phase-3 subsystem timing buckets |
| `Justfile` `samply-doc` / `samply-corpus` / `samply-render` | one-line wrappers |
| `docs/adr/0014-phase-breakdown-findings.md` | original phase 3 outlier finding (`明治人物月旦`) |
| `docs/adr/0013-aozora-scan-leading-byte-strategy-loses-on-japanese.md` | T1 architectural decision |
