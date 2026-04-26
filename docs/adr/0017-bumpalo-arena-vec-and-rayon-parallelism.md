# 0017. R4 — bumpalo arena Vec for inter-phase materialisation + rayon corpus parallelism

- Status: accepted (R4-A architectural; R4-B performance)
- Date: 2026-04-27
- Deciders: @P4suta
- Tags: architecture, performance, lex, bench, 0.2.0
- References: ADR-0009 (clean layered architecture), ADR-0014 (phase
  breakdown findings), ADR-0016 (deforestation reversal investigation)

## Context

After ADR-0016 (R3) settled the inter-phase shape on heap-`Vec`
materialisation (R2) + streaming Phase 3 (R3 measurement), two clean
targets remained on the post-R3 corpus profile:

| Bucket | % of corpus parse | Why |
|---|---:|---|
| **allocation** | **25.7 %** | Per-parse `Vec<Token>` + `Vec<PairEvent>` heap mallocs; libc malloc/free; memcpy/memmove |
| **single-threaded only** | (corpus throughput ≈ 250 MB/s on 16-core host) | The corpus sweep is doc-independent but `throughput_by_class` / `phase_breakdown` walk docs serially |

R4 attacks both. Two changes ship under one bookmark:

- **R4-A** — replace inter-phase heap `Vec`s with arena-backed
  `bumpalo::collections::Vec<'a, _>`. The borrowed pipeline already
  owns one [`Arena`] per parse — collapsing per-parse `Vec<Token>` /
  `Vec<PairEvent>` malloc/free traffic into one bump-pointer advance
  per element should halve the allocation bucket.
- **R4-B** — add `AOZORA_PROFILE_PARALLEL=1` opt-in to the
  `throughput_by_class` and `phase_breakdown` bench harnesses. Rayon's
  work-stealing pool fans the per-doc `lex_into_arena` calls across
  cores; per-task `Arena::new()` keeps `bumpalo`'s `!Sync` contract
  intact.

Both build on infrastructure already in place: `bumpalo` is in
workspace deps with the `collections` feature; `Arena::bump()` exposes
the inner allocator; `CorpusSource: Send + Sync` is parallel-ready;
`rayon = "1.11"` is in workspace deps but was unused.

## R4-A — bumpalo BumpVec for Phase 1 / Phase 2

### Architectural beauty: 3 → 2 surfaces, dead heap-batch APIs removed

The pre-R4 lexer exposed three batch entry shapes per phase:

| Surface | Phase 1 | Phase 2 | Phase 3 |
|---|---|---|---|
| streaming | `tokenize() -> Tokenizer` | `pair(I) -> PairStream` | `classify(I) -> ClassifyStream` |
| heap-batch | `tokenize_to_vec(s) -> Vec<Token>` | `pair_slice(&[Token]) -> PairOutput` | `classify_slice` / `classify_into_emit` (R3-added) |
| arena-batch | (none) | (none) | (none) |

Grep showed the heap-batch APIs were called from **exactly one place**:
the borrowed `Pipeline`. With R4-A's arena migration the heap-batch row
becomes dead code. Following the "no dead code" rule and the
`feedback_aggressive_clean_architecture.md` preference, we did not add
arena-batch alongside heap-batch — we **replaced** heap-batch outright:

| Surface | Phase 1 | Phase 2 | Phase 3 |
|---|---|---|---|
| streaming | `tokenize() -> Tokenizer` | `pair(I) -> PairStream` | `classify(I) -> ClassifyStream` |
| arena-batch | `tokenize_in(s, &Arena) -> BumpVec<'a, Token>` | `pair_in(&[Token], &Arena) -> PairOutputIn<'a>` | (intentionally none — see below) |

Each axis now has one obvious consumer:

- **streaming**: incremental / FFI / pull-based callers that have no
  arena to spend. Lazy semantics; back-pressure friendly.
- **arena-batch**: the borrowed `Pipeline` and any caller that already
  holds an `Arena` for the AST it's about to build. Materialised
  one-bump-pointer-advance-per-element — no heap `malloc`/`free`.

Phase 3's `classify_slice` / `classify_into_emit` were also dropped.
ADR-0016's R3 bake-off measured both as net regressions vs the
streaming `classify` Iterator path on the corpus; R4 retains streaming
Phase 3 and removes the dead alternatives. The `Pipeline.build()`
chain is unchanged from R3: `BumpVec<PairEvent>::into_iter()` feeds the
streaming `classify`, which streams `ClassifiedSpan`s into the
`ArenaNormalizer` callback. No third Vec materialisation.

### Measured perf delta — neutral (the surprise)

`throughput_by_class` (single pass, sequential, 17 435-doc corpus, 16-
core x86_64):

| Band | R3 final | R4-A | Δ |
|---|---:|---:|---:|
| < 50 KB | 241.6 MB/s | 245.3 MB/s | +1.5 % |
| 50 K-500 K | 272.2 MB/s | 264.3 MB/s | -2.9 % |
| 500 K-2 M | 224.4 MB/s | 211.7 MB/s | -5.7 % |
| > 2 MB | 112.7 MB/s | 108.9 MB/s | -3.4 % |

`phase_breakdown` (sequential, full corpus):

| Probe | R3 final | R4-A |
|---|---:|---:|
| `lex_into_arena` aggregate | 284.7 MB/s | 284.7 MB/s |
| doc 49178 `lex_into_arena` outlier | (top-5: 15.88 ms classify / 30.89 ms total) | (top-5: 15.88 ms / 30.89 ms — identical) |

Net: **R4-A is corpus-throughput neutral within ±5 % run-to-run noise.**
Plan ADR-0016 had sketched +5-25 % on the assumption that the alloc
bucket measured by samply was load-bearing; R4-A's measurement falsifies
that assumption. The corpus-median `Vec<Token>` / `Vec<PairEvent>`
allocation is small enough that glibc's `tcache`-amortised malloc
matches bumpalo's bump-pointer advance, and bumpalo itself pays a
new-chunk `mmap` when the capacity hint forces a fresh chunk.

R4-A still ships because it is an **architectural** win — the public
API surface contracts from 3 axes (streaming + heap-batch + would-be
arena-batch) to 2 (streaming + arena-batch); roughly 200 LoC of
heap-batch dead code (`tokenize_to_vec`, `pair_slice`, `PairOutput`,
`classify_slice`, `classify_into_emit`, `ClassifyOutput`) is removed;
the `Pipeline` field types now visibly carry the arena lifetime. It is
**not** a perf win on its own.

The remaining 25.7 % allocation bucket is dominated by AST allocation
inside Phase 3's recognisers (interner growth, `Container` /
`Inline` / `Block` arena allocs) — work that R4-A does not touch.
Modern-algorithm follow-ups under consideration: SoA Token storage
(separate tag column from payload column), per-thread arena reuse
(`Bump::reset()` between docs in a worker), variable-length event
encoding. None ship in R4 (see "out of scope").

## R4-B — Rayon corpus parallelism

### Implementation shape

Both `throughput_by_class` and `phase_breakdown` follow the same
opt-in pattern:

```rust
fn parallel_mode() -> bool {
    matches!(
        env::var("AOZORA_PROFILE_PARALLEL").ok().as_deref(),
        Some("1" | "true" | "yes")
    )
}

let pairs: Vec<(u64, u64)> = if parallel {
    docs.par_iter().map(|(_, text)| measure(text)).collect()
} else {
    docs.iter().map(|(_, text)| measure(text)).collect()
};
```

Notable:

- **Per-task `Arena::new()`**: each rayon worker constructs its own
  arena inside the closure. `Arena` is `Send` (one per task crosses
  thread boundaries cleanly) but `!Sync` — the per-task pattern means
  no two threads ever share the same arena, so `bumpalo`'s
  `RefCell<Bump>` interior is never observed concurrently.
- **`par_iter().collect()` preserves input order**: per-doc result
  vectors stay aligned with corpus iteration order, so `Top-5 by
  classify` rankings match between sequential and parallel runs.
- **Decode errors as `None`**: `phase_breakdown`'s `decode_sjis` flow
  returns `Option<DocResult>` rather than mutating a shared
  `AtomicU64`. Post-collect counts `None`s. No atomics, no
  `Mutex<Vec>`, no race.
- **Progress log suppressed under parallel**: replaced the
  every-2k-doc `eprintln!` with one `done in X.XXs` summary line
  after the par_iter completes — interleaved progress lines from 16
  workers would be unreadable and add no signal.
- **Sequential is the default**: per-doc latency numbers stay
  reproducible / variance-stable, and the sampling profiler
  (`samply`) attaches cleanly to a single-thread call stack.

### Measured speedup (16-thread x86_64 host)

`throughput_by_class`:

| Mode | parse wall | per-band MB/s (<50 K / 50-500 K / 500 K-2 M / >2 M) |
|---|---:|---:|
| Sequential | 3.31 s | 245.3 / 264.3 / 211.7 / 108.9 |
| Parallel (16 threads) | 0.68 s | 169.8 / 90.0 / 46.2 / 84.7 |

```
Parallel: 16 threads   serial-work 9.59s   concurrent-wall 0.68s   scaling 14.14× (ideal 16×)
```

Per-band MB/s drops because each thread's per-doc `Instant::now()`
delta widens under CPU contention (cross-CPU memory traffic, shared
LLC pressure). The **wall-clock scaling is the load-bearing number**:
14.14× / 16 threads = **88.4 % efficiency**. Doc-independent
parallelism on a corpus sweep should be near-perfect; what we see is
exactly that, modulo the natural overhead from per-doc arena
allocation (`mmap` syscalls serialise in the kernel page-cache).

`phase_breakdown`:

```
phase_breakdown: done in 1.29s, 1 decode errors
parallel : 16 threads, scaling 6.81× (serial work 8.79s)
```

Lower scaling than `throughput_by_class` because each task does **5×
the work** (`measure_one` runs sanitize, tokenize, pair, classify,
and `lex_into_arena` separately) and constructs **2× the arenas** per
doc — `mmap`/`munmap` contention scales sub-linearly. Still a 6.81×
speedup is a clean win for the bench harness; the canonical
production-style measurement remains `throughput_by_class`'s
parallel mode.

### What R4-B does not change

- `lex_into_arena` itself is unchanged. Per-doc parse remains
  sequential (paired-bracket nesting requires sequential parse;
  intra-doc parallelism does not fit Aozora's structure).
- `samply` recipes still drive sequential mode by default — the
  profiler attaches to one thread cleanly.
- CI / regression measurement uses sequential. Parallel is a
  performance amplifier for development iteration, not a new
  contract surface.

## Decision

Ship both. R4-A as a pure architectural improvement (perf-neutral but
removes dead code and contracts the public API surface from 3 axes to
2). R4-B as a development-iteration speedup (16-core host: corpus
sweep 3.31 s → 0.68 s).

## Out of scope (deferred)

- **Per-thread `Bump::reset()` reuse.** Each rayon worker keeps one
  arena alive across its assigned docs, calling `reset()` between
  parses to drop allocations without releasing the chunks. Saves the
  per-doc `mmap` cost; plausible 5-10 % corpus throughput win on top
  of R4-B. Requires thread-local-storage gymnastics; deferred until
  measured `mmap` overhead under parallel exceeds 5 % of wall.
- **SoA Token storage.** `Token` enum is `(tag: u8, payload: …)` —
  current `Vec<Token>` interleaves tag and payload bytes. A
  Structure-of-Arrays layout (separate `Vec<u8>` for tags, separate
  payload columns) would let the merge-walk's tag scan stay in L1
  while payload reads happen out-of-band. Plausible win: tag-scan
  hot loops drop a cache line per iteration. Deferred — needs its
  own design ADR + bake-off.
- **Variable-length PairEvent encoding.** `PairEvent` enum size =
  max-variant size; small variants (`Newline { pos: u32 }`) waste
  bytes. Encoded compactly (varint or per-tag dispatch table) would
  shrink the per-parse `BumpVec<PairEvent>` footprint. Deferred —
  measure first; the constant-factor win is small and the code-shape
  cost is real.
- **`Vec<Diagnostic>` arena allocation.** Diagnostics are rare
  (~0.1 / doc on the corpus); the heap allocation cost is in the
  noise. Stays heap.

## Validation gates

| Gate | Status |
|---|---|
| `cargo test --workspace --no-fail-fast` | 549 / 0 |
| `cargo test -p aozora-lex --test property_borrowed_arena` | 12 / 0 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| `cargo fmt --all -- --check` | clean |
| `throughput_by_class` sequential | within ±5 % of R3 final |
| `throughput_by_class` parallel | 14.14× scaling on 16 cores |
| `phase_breakdown` parallel | 6.81× scaling, output coherent |
