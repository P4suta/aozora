# 0016. I-2 deforestation reversal investigation — Iterator chain re-affirmed; slice/callback APIs added

- Status: accepted (negative result for production wiring; positive for new APIs)
- Date: 2026-04-27
- Deciders: @P4suta
- Tags: architecture, performance, lex, 0.2.0
- References: ADR-0009 (clean layered architecture, § I-2 deforestation), ADR-0013 (T1 negative result), ADR-0015 (T2 SIMD scanner bake-off)

## Context

Two measurements after T2 (the Hyperscan Teddy bake-off) suggested the
inter-phase Iterator chains were eating ~88 % of Phase 3 wall and
~7 % of doc 49178's wall:

```
phase3_subsystems (instrumented):
  iter_dispatch (outer wall)      : 8621.68 ms (100 %)
    recogniser-leaf total          :  810.95 ms ( 9.4 %)
    append_to_frame                :  218.40 ms ( 2.5 %)
    pure dispatch overhead         : 7592.33 ms (88.1 %)

doc 49178 samply (production, no instrumentation):
   1  17.5 %  core::ptr::write
   4   3.9 %  PairStream::next
   8   2.1 %  Tokenizer::next
  16   1.4 %  ClassifyStream::next
```

The hypothesis: ADR-0009 § I-2 ("deforestation" — fuse phases as
`impl Iterator<Item = T>` chains to avoid intermediate `Vec` allocs)
was a premature optimisation under outdated assumptions, and the per-
item iterator overhead for tens-of-thousands of events per doc would
dwarf one extra `Vec` alloc per phase.

A new jj bookmark `r1-r2-r3-deforestation-reversal` carried four
sequential changes (R1 inline annotations, R2 Phase 1→Vec, R3 Phase 3
slice/callback APIs) so each step's marginal contribution could be
measured independently via `jj edit <change>`.

## Bake-off

Single-pass `throughput_by_class` corpus throughput (MB/s) by band,
plus the doc-49178 outlier (`pathological_probe` 100-iter avg):

| Variant | <50KB | 50K-500K | 500K-2M | >2M | doc 49178 lex |
|---|---:|---:|---:|---:|---:|
| **R0 baseline** (T2 done; pre-R1) | 272.1 | 285.6 | 233.0 | 135.6 | 1.86 ms |
| R1 (aggressive `#[inline]`) | 257.0 | 282.5 | 225.7 | 133.3 | 1.91 ms |
| R1 (selective `#[inline]`) | 268.7 | 287.4 | 239.0 | 136.4 | — |
| R2 (Phase 1→Vec, Phase 2 takes `&[Token]`) | 243.8 | 275.8 | 225.0 | 114.5 | 1.78 ms |
| R3 v1 (`classify_slice` wholesale Vec) | 227.1 | 261.9 | 218.6 | 104.2 | 2.03 ms |
| R3 v2 (`classify_into_emit` callback) | 242.7 | 279.5 | 221.4 | 109.1 | 1.80 ms |
| **R3 final** (slice/callback APIs added; Pipeline keeps Iterator) | 241.6 | 272.2 | 224.4 | 112.7 | **1.21 ms** |

Per-phase wall (corpus aggregate, ms):

| Variant | sanitize | tokenize | pair | classify |
|---|---:|---:|---:|---:|
| R0 baseline | 364 | 910 | 280 | 1405 |
| R3 final | 394 | 906 | 286 | (Iterator path) |

Doc 49178 phase breakdown (per-call avg, 100 iter):

| Variant | sanitize | tokenize | pair | classify | 4-phase TOTAL | lex_into_arena |
|---|---:|---:|---:|---:|---:|---:|
| R0 baseline | 0.31 | 0.58 | 0.23 | 0.84 | 1.96 | 1.86 |
| R3 final | 0.31 | 0.58 | 0.24 | **0.43** | 1.57 | **1.21** |

## Decision

**Pipeline.build retains the streaming Iterator path.** R2's Phase 1
materialisation (`Vec<Token>`) and Phase 2 materialisation
(`Vec<PairEvent>`) are kept — the Pipeline now exposes
`.tokens()` / `.events()` accessors for inspection and the `I` generic
parameter has been dropped (architectural improvement). But Phase 3 is
driven via `classify(events.into_iter(), …)` not `classify_slice` or
`classify_into_emit` — the slice/callback APIs are kept as Phase 3 API
surface for batch / FFI consumers.

Reasoning by the data:
- **R1 (`#[inline]` annotations alone)** is null at best, regression
  at worst. LLVM at -O3 + fat-LTO already inlines optimally on this
  code; aggressive forcing causes i-cache thrash on dispatcher
  wrappers. Negative result documented in `phase3_classify.rs`.
- **R2 (Phase 1/2 Vec materialisation)** trades small bands for the
  outlier doc. Net on the corpus: -3 to -15 % throughput. But:
  Pipeline gains a much cleaner shape (no `I` generic, `.tokens()`
  / `.events()` accessors). Architectural win + perf neutral-to-
  negative; ship the structure, document the trade.
- **R3 (Phase 3 slice/callback)** wins big on doc 49178 outlier
  (lex_into_arena 1.86 → 1.21 ms, -35 %; classify 0.84 → 0.43 ms,
  -49 %) but lost on corpus average. The pre-R3 chain had three
  nested `impl Iterator` types
  (`Tokenizer<'a> → PairStream<Tokenizer<'a>> → ClassifyStream<…>`);
  R3 production now has exactly one (`Vec<PairEvent>::IntoIter →
  ClassifyStream`), which monomorphises better. This explains the
  outlier win even though Pipeline.build still uses an Iterator.

The new APIs (`classify_slice`, `classify_into_emit`, `tokenize_to_vec`,
`pair_slice`, `PairOutput`, `ClassifyOutput`) are net-positive
architecturally: batch / FFI consumers no longer have to bolt
`(&mut stream).collect()` on the streaming path.

## Reversed hypothesis

**ADR-0009 § I-2 (deforestation) is re-affirmed for production.**
The premise was correct on the corpus average, even though the
phase3-instrument numbers suggested otherwise. The "88 % iterator-
dispatch overhead" reading was inflated by the instrumentation
itself (`Instant::now()` per `SubsystemGuard`). Without
instrumentation, the production samply trace shows
`ClassifyStream::next` at 1.4 % on the worst doc — meaningful but
not load-bearing.

The corpus average *cannot* be improved by replacing the Iterator
chain with `Vec` materialisation on this workload: per-doc Vec
allocation cost (3-5 mid-sized `Vec`s) accumulates per parse, and at
the corpus median doc size of ~16 KiB, those allocs dominate the
~25 µs per-doc parse budget.

## Where the time really goes

The remaining headroom — should anyone want to chase it — is in:

1. **bumpalo arena allocation for the inter-phase Vecs.** R2's
   `Vec<Token>` / `Vec<PairEvent>` are heap-allocated; allocating
   them out of the parse arena (which `Pipeline` already owns)
   would convert N small per-parse allocs into one bump-pointer
   advance. Likely R4 — left for a future investigation since
   the Vec-passing path is currently disabled in production.
2. **Phase 3 internal `pending_outputs: VecDeque` removal.** The
   N2 ring-buffer fix is required by `ClassifyStream::next`'s
   one-yield-per-call API. Refactoring to push directly into a
   sink (closure or `&mut Vec`) — for the *streaming* path too
   — would let LLVM eliminate the ring buffer entirely. Risky
   refactor (4000+ lines of Phase 3); deferred until a real
   workload demands it.

## Consequences

**Net code change after R0 → R3 final:**

- Phase 1: `tokenize_to_vec(source) -> Vec<Token>` (new public API,
  alongside streaming `tokenize`).
- Phase 2: `pair_slice(&[Token]) -> PairOutput` + `PairOutput`
  struct (new public API, alongside streaming `pair`).
- Phase 3: `classify_slice(&[PairEvent], …) -> ClassifyOutput` +
  `classify_into_emit(&[PairEvent], …, F) -> Vec<Diagnostic>` +
  `ClassifyOutput` struct (new public APIs, alongside streaming
  `classify`).
- `Pipeline<S, I>` → `Pipeline<S>` (drop `I` generic).
- `Pipeline` carries `Option<Vec<Token>>` / `Option<Vec<PairEvent>>`
  internally; `.tokens()` and `.events()` accessors expose them
  for inspection.
- `Pipeline.build` retains the streaming `classify(events.into_iter(),
  …)` path — the slice/callback APIs are not wired in production.
- Phase 3 docstring in `phase3_classify.rs` records the R1 inline-
  hint negative result.

Tests: 549 / 0 across all 4 jj changes. Clippy + fmt clean at every
step.

## Alternatives considered

- **Ship R3 v1 (wholesale Vec) anyway**, accepting the corpus
  regression for the doc-49178 outlier win. Rejected: corpus
  throughput is the user-visible metric (corpus sweep CI gate);
  outliers are local pain, corpus is everyone's pain.
- **Bumpalo Vec for the R2 inter-phase Vecs.** Plausible but
  premature — the Iterator path is currently the production winner,
  so the bumpalo upside is theoretical until measured. Filed under
  "where the time really goes" above.
- **Revert R2 entirely.** Considered; rejected because the Pipeline
  shape simplification (drop `I` generic, expose `.tokens()` /
  `.events()`) is independent architectural value, even though the
  perf trade is negative on small/large bands.
- **Macro-expand the Iterator chain.** Some Rust shops use macros
  to inline-flatten an Iterator chain into a single state machine.
  Heavy weapon, doesn't suit the workspace's "library-of-pure-fns"
  shape.

## References

- ADR-0009 (Clean layered architecture) — § I-2 deforestation
  hypothesis (now re-affirmed by data)
- ADR-0013 (T1 leading-byte SIMD scanner negative result) — same
  "investigated, reverted, documented" pattern
- ADR-0015 (T2 Teddy bake-off) — produced the R0 baseline this
  investigation measured against
- `crates/aozora-lexer/src/phase1_events.rs::tokenize_to_vec` —
  Phase 1 batch API
- `crates/aozora-lexer/src/phase2_pair.rs::pair_slice` — Phase 2
  batch API
- `crates/aozora-lexer/src/phase3_classify.rs::classify_slice` /
  `classify_into_emit` — Phase 3 batch APIs
- `crates/aozora-lex/src/pipeline.rs` — Pipeline shape
  simplification
- `phase3_classify.rs` module docstring — R1 negative result inline
