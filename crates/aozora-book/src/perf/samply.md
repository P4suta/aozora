# Profiling with samply

[`samply`](https://github.com/mstange/samply) is the workspace's
sampling profiler. It produces `.json.gz` traces in the
[Firefox-Profiler](https://profiler.firefox.com/) gecko format
that can be loaded into the web UI for visual analysis, or fed to
the in-tree `aozora-trace` crate for automated rollups.

## Quick commands

```sh
# Single corpus document
AOZORA_CORPUS_ROOT=/path/to/corpus \
  just samply-doc 001529/files/50685_ruby_67979/50685_ruby_67979.txt

# Full corpus, parser-bound (5 parse passes after the one-time load)
AOZORA_CORPUS_ROOT=/path/to/corpus just samply-corpus

# Full corpus, render-bound
AOZORA_CORPUS_ROOT=/path/to/corpus just samply-render

# Open in Firefox-Profiler
samply load /tmp/aozora-corpus-<timestamp>.json.gz
```

All three are wrappers over the `aozora-xtask samply` subcommand,
which:

- Builds the bench probe with `--profile=bench` (debug info preserved).
- Runs samply against the resulting binary.
- Drops the `.json.gz` in `/tmp/`.

## Why these run on the host (not Docker)

samply uses `perf_event_open(2)` for kernel sampling. Docker's
default seccomp profile blocks that syscall. The xtask binary
therefore runs on the host (not via `docker compose run`) and the
Justfile recipes are exempt from the workspace's normal
"everything in Docker" policy.

The recipes check `/proc/sys/kernel/perf_event_paranoid` on entry
and print the fix-up command if the value is too high (default 2;
needs to be ≤ 1 for unprivileged sampling):

```sh
echo 1 | sudo tee /proc/sys/kernel/perf_event_paranoid
```

## Why `--profile=bench` and not `--release`

`cargo build --release` uses `[profile.release]`, which has
`debug = 0` + `strip = "symbols"`. Samply still records samples,
but they show up as raw addresses (`0x8fb61`) instead of function
names — every sample becomes useless to a human reader.

The workspace `[profile.bench]` inherits from release but sets
`debug = 1` + `strip = "none"`. The xtask wrappers automatically
build with `--profile=bench`. If you launch samply manually, do the
same.

## Corpus load dominates a single-pass trace

`throughput_by_class` and `render_hot_path` spend most wall time in
Shift_JIS decode + filesystem I/O during the one-time corpus load.
A single-pass samply trace puts `__memmove_avx_unaligned` and
`encoding_rs::ShiftJisDecoder` at the top — *not* the parser.

Fix: set `AOZORA_PROFILE_REPEAT=K` (or pass `K` to
`just samply-corpus`) so the parse pass runs `K` times after the
load. The xtask defaults to 5; raise to 10+ for very small corpora.

## Trace analysis from the CLI

`aozora-xtask trace …` (and the `just trace-*` shortcuts) load
saved `.json.gz` traces, symbolicate them via the `aozora-trace`
crate (DWARF lookup is pure-Rust through `addr2line::Loader`), and
run the bundled analyses.

```sh
# 1. One-time per trace: write the symbol cache next to it
just trace-cache /tmp/aozora-corpus-<ts>.json.gz

# 2. Analyses (cache is auto-loaded if present)
just trace-libs    /tmp/aozora-corpus-<ts>.json.gz                  # binary vs libc vs vdso
just trace-hot     /tmp/aozora-corpus-<ts>.json.gz 25               # top-25 hot leaf frames
just trace-rollup  /tmp/aozora-corpus-<ts>.json.gz                  # bucketed by aozora's built-in categories
just trace-stacks  /tmp/aozora-corpus-<ts>.json.gz 'teddy' 5        # full call chains hitting any frame matching `teddy`
just trace-compare /tmp/before.json.gz /tmp/after.json.gz 25        # before/after diff
just trace-flame   /tmp/aozora-corpus-<ts>.json.gz | flamegraph.pl > flame.svg
```

Each analysis returns a typed report — `HotReport`, `LibraryReport`,
`RollupReport`, `ComparisonReport`, `MatchedStacksReport`,
`FlameReport` — whose module docstring explains the algorithm.

## Why a pure-Rust DWARF symbolicator?

The mainstream alternative is shelling out to `addr2line(1)` from
binutils. We don't because:

- Process spawn cost. A typical trace has 5 000+ unique addresses;
  spawning `addr2line` per address is unworkable. Pipelining
  through a single subprocess works but ties symbolisation to the
  presence of binutils on `PATH` (not always true on minimal
  containers).
- Build-id verification. The `aozora-trace::Symbolicator` checks
  the binary's `gnu-build-id` against the trace's `codeId` so
  rebuilding between recording and analysis fails loudly rather
  than producing wrong symbol names. `addr2line(1)` has no such
  check.
- Caching. The symbolicator writes a sidecar `<trace>.symbols.json`
  on first call (~100 ms per binary) and reads from it on every
  subsequent call (instant). Re-running `addr2line` per analysis
  would re-walk DWARF every time.

## Verifying the SIMD scanner is firing

```rust
// In any binary or test
println!("{}", aozora_scan::best_scanner_name());
// "teddy" | "hoehrmann-dfa" | "memchr-multi"
```

Or under samply, look for `aozora_scan::backends::teddy::scan_offsets`
in the trace's call tree. If the trace shows
`memchr::arch::x86_64::avx2::*` instead, you're on the scalar
fallback (which uses memchr's own SIMD dispatch internally — still
SIMD, just not aozora-scan's).

## Workflow recipes

### "I changed something, did I regress?"

```sh
# Microbench the per-band tokenizer throughput
cargo bench -p aozora-lex --bench tokenize_compare

# Macrobench the full pipeline end-to-end
AOZORA_CORPUS_ROOT=… cargo run --release --example throughput_by_class -p aozora-bench
AOZORA_CORPUS_ROOT=… cargo run --release --example render_hot_path     -p aozora-bench

# Check the worst doc didn't regress
AOZORA_CORPUS_ROOT=… AOZORA_PROBE_DOC=000286/files/49178_ruby_58807/49178_ruby_58807.txt \
  cargo run --release --example pathological_probe -p aozora-bench
```

### "Where is `lex_into_arena` spending its time?"

```sh
# Macroscopic per-phase split
AOZORA_CORPUS_ROOT=… cargo run --release --example phase_breakdown -p aozora-bench

# Latency tail shape
AOZORA_CORPUS_ROOT=… cargo run --release --example latency_histogram -p aozora-bench

# Microscopic: which classify recogniser dominates a specific doc?
AOZORA_CORPUS_ROOT=… AOZORA_PROBE_DOC=… \
  cargo run --release --features instrument --example pathological_probe -p aozora-bench
```

## See also

- [Benchmarks](bench.md) — the per-probe descriptions.
- [Corpus sweeps](corpus.md) — corpus setup and `AOZORA_*` env vars.
