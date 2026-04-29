# Profiling guide

How to take a samply profile of `aozora`, what the bench probes
report, and the pitfalls that catch first-time profilers.

For *how to use* the parser, see [`USAGE.md`](./USAGE.md). For the
overall design, see [`ARCHITECTURE.md`](./ARCHITECTURE.md).

---

## Quick start

The two commands you'll use most:

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
| `fused_vs_materialized` | Does the deforestation actually win? | per-band gap % between fused (lex_into_arena) and materialized (per-phase collect) |
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
Shift_JIS decode + filesystem I/O during the one-time corpus load.
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
recording. The `Symbolicator` also checks the binary's
`gnu-build-id` against the trace's `codeId` so rebuilding between
recording and analysis fails loudly rather than producing wrong
symbol names.

### 6. Subsystem instrumentation distorts the numbers it reports

`phase3-instrument` wraps every recogniser entry in a
`SubsystemGuard` that calls `Instant::now()` on construction +
drop. For the dominant inner-loop recognisers this adds enough
overhead that the **report's own timing is significantly skewed**.
Use the instrumentation to compare *relative* costs between
subsystems, not as an absolute number. For absolute numbers, run
`phase_breakdown` (no instrumentation).

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

### "Analyse a saved samply trace from the CLI"

`aozora-xtask trace ...` (and the `just trace-*` shortcuts) load
saved `.json.gz` traces, symbolicate them via the `aozora-trace`
crate (DWARF lookup is pure-Rust through `addr2line::Loader`), and
run the bundled analyses. A sidecar `<trace>.symbols.json` caches
resolved labels — first call is slow (~100 ms per binary),
subsequent ones are instant.

```bash
# 1. One-time per trace: write the symbol cache next to it.
just trace-cache /tmp/aozora-corpus-<ts>.json.gz

# 2. Analyses (cache is auto-loaded if present):
just trace-libs    /tmp/aozora-corpus-<ts>.json.gz                  # binary vs libc vs vdso
just trace-hot     /tmp/aozora-corpus-<ts>.json.gz 25               # top-25 hot leaf frames
just trace-rollup  /tmp/aozora-corpus-<ts>.json.gz                  # bucketed by aozora's built-in categories
just trace-stacks  /tmp/aozora-corpus-<ts>.json.gz 'teddy' 5        # full call chains hitting any frame matching `teddy`
just trace-compare /tmp/before.json.gz /tmp/after.json.gz 25        # before/after diff
just trace-flame   /tmp/aozora-corpus-<ts>.json.gz | flamegraph.pl > flame.svg
```

Each analysis returns a typed report (`HotReport`, `LibraryReport`,
`RollupReport`, `ComparisonReport`, `MatchedStacksReport`, …) whose
module docstring explains the algorithm.

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

### "Confirm the SIMD scanner is firing"

```rust
// In any binary or test:
println!("{}", aozora_scan::best_scanner_name());
// Prints "avx2" / "teddy" / "scalar" — pure inspection, no SIMD work.
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
| `crates/aozora-xtask/src/main.rs` | `xtask samply <doc \| corpus \| render>` |
| `crates/aozora-xtask/src/trace.rs` | `xtask trace <cache \| hot \| libs \| rollup \| stacks \| compare \| flame>` |
| `crates/aozora-trace/` | pure-Rust trace loader + symbolicator + analyses |
| `crates/aozora-lexer/src/instrumentation.rs` | the 17 phase-3 subsystem timing buckets |
| `Justfile` `samply-doc` / `samply-corpus` / `samply-render` | one-line wrappers |
