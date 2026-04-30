# Corpus sweeps

aozora's tier-A acceptance gate is a **corpus sweep**: every Aozora
Bunko work parses without panicking, and the
`parse ∘ serialize ∘ parse` round-trip is stable. The corpus has
~17 000 works in active rotation; sweeping the lot takes ~90 s on a
modern x86_64 desktop.

## Setting up the corpus

`AOZORA_CORPUS_ROOT` should point at a directory containing the
unpacked Aozora Bunko tarball:

```text
$AOZORA_CORPUS_ROOT/
├── 000001/
│   └── files/
│       └── 18310_ruby_01058/
│           └── 18310_ruby_01058.txt   ← Shift_JIS .txt source
├── 000002/
│   └── files/
│       └── …
└── …
```

The structure mirrors the upstream
[aozorabunko](https://github.com/aozorabunko/aozorabunko) repo. Set
the env var once in your shell:

```sh
export AOZORA_CORPUS_ROOT=/path/to/aozorabunko
```

Every probe, every sample-profile recipe, and the corpus sweep test
suite reads it.

## Running the sweep

```sh
just corpus-sweep
```

Wraps the `aozora-corpus` crate's `ParallelSweep` runner. Iterates
every `.txt` file under `$AOZORA_CORPUS_ROOT`, parses it, verifies:

- No panic.
- `tree.diagnostics()` count is within an expected envelope.
- `parse(serialize(parse(source))) == parse(source)` (round-trip
  property).
- Render emits valid UTF-8 HTML (no broken byte sequences).

Failure: prints the offending document path + diagnostic, exits
non-zero.

## Why blake3 / zstd for the archive variant?

`aozora-corpus` ships an *archive* mode: the corpus packed into a
single `.zst` file with a blake3 manifest. This is what CI uses
(the corpus is downloaded once per workflow run and unpacked
in-memory).

- **blake3** for per-entry content-addressed hashing. Used so the
  archive packer can detect "this work hasn't changed since the
  last build" and skip re-encoding it. blake3 over sha256: ~10×
  faster on the same data, no security trade-off for our use case
  (we're not signing anything, just diffing).
- **zstd** for compression. Frame-level random access matters
  because the `ParallelSweep` runner wants to mmap individual works
  on demand without decompressing the whole archive. zstd over gzip
  / xz: 5–10× faster decompression at comparable ratios.

Both crates are mainstream pure-Rust APIs (the underlying libzstd
is C, but the boundary is hidden behind the `zstd` crate's safe API).

## Why parallel sweep?

A serial sweep runs sequentially through every work; on a 16-core
machine that's wall-clock 16× the per-doc parse time. The
`ParallelSweep` runner uses `rayon` to parse documents in parallel,
sized to *physical* cores via `num_cpus::get_physical()` — not
logical cores.

The reason is memory bandwidth. The parser is bandwidth-bound, not
ALU-bound (the SIMD scanner streams the source through L1 once per
trigger byte, then the lexer touches each token a few more times).
SMT siblings starve each other for cache lines and bus bandwidth, so
oversubscribing logical cores actively slows the sweep. Sized to
physical, the throughput peaks where the bandwidth ceiling does.

## `posix_fadvise(POSIX_FADV_DONTNEED)` for honest cold-cache numbers

The `xtask corpus uncache` command evicts every corpus file from
the kernel page cache before a measurement run:

```sh
cargo run -p aozora-xtask --release -- corpus uncache
```

It uses `posix_fadvise(fd, 0, 0, POSIX_FADV_DONTNEED)` per file —
no sudo required (unlike `echo 1 > /proc/sys/vm/drop_caches`, which
needs root and drops *every* cache, defeating the purpose).

Why this matters: a "fresh" benchmark run that finds the corpus
already warm in the page cache reports throughput numbers that no
cold start can ever achieve. The `uncache` step makes "cold
benchmark" a real, repeatable thing.

## Probes that go corpus-wide

| Probe | What |
|---|---|
| `throughput_by_class` | Per-band MB/s for `lex_into_arena`. Splits the corpus by document size (small / medium / large / huge). |
| `phase_breakdown` | Per-phase ms per doc. |
| `latency_histogram` | Log-bucketed latency distribution per phase. |
| `diagnostic_distribution` | What fraction of docs emit diagnostics? Histogram by diag count. |
| `allocator_pressure` | Arena bytes / source byte ratio + intern dedup ratio. |
| `render_hot_path` | Per-band render MB/s. |

See [Benchmarks](bench.md) for the full list.

## Why a dedicated `aozora-corpus` crate?

Three concerns kept apart from `aozora-bench`:

1. **Corpus discovery and loading.** Walking the directory, decoding
   Shift_JIS, applying any per-work filters. This is shared by every
   probe + by the xtask corpus pack/unpack tooling.
2. **Archive format.** The blake3 + zstd packing/unpacking lives
   here so the bench harness doesn't pull in compression libraries.
3. **Parallel sweep runner.** A reusable `rayon::par_iter` wrapper
   with the right ordering (largest documents first to balance load).

`aozora-bench` then builds on this — each probe is a thin
`for doc in corpus { measure(doc) }` loop, with the corpus crate
handling all the I/O.

## Why a separate `AOZORA_PROFILE_REPEAT`?

samply traces of probes that include corpus loading get dominated
by I/O and Shift_JIS decode (see
[Profiling with samply](samply.md#corpus-load-dominates-a-single-pass-trace)).
Running the parse pass `K` times per document after the one-time
load gives samply enough parse-bound wall time to catch the
parser hot frames. Default `K = 5`; raise to 10+ for very small
corpora.

## See also

- [Benchmarks](bench.md) — the per-probe descriptions.
- [Profiling with samply](samply.md) — the trace workflow.
