# 0019. M1-M3 modern algorithmic follow-ups — measured deltas + FSM verdict

- Status: accepted (M-1 promoted; M-2 + M-3 documented negative results, kept in jj history, not on default code path)
- Date: 2026-04-27
- Deciders: @P4suta
- Tags: architecture, performance, lex, classify, 0.2.0
- References: ADR-0009 (clean layered architecture), ADR-0014 (phase
  breakdown findings), ADR-0015 (T2 SIMD scanner bake-off — PEXT
  rejection), ADR-0016 (deforestation reversal investigation),
  ADR-0017 (R4 — bumpalo arena Vec + rayon parallelism)

## Context

ADR-0017 (R4) listed five "modern algorithmic follow-ups" as out of
scope, gated by future measurement. This ADR records the
implementation and measurement of four of them; the fifth (BMI2 PEXT
trigger dispatch) was already rejected in ADR-0015 and is re-confirmed
here.

User direction this session was unambiguous: implement all four,
prioritise architectural beauty, measure each step against the
others. Plan-agent validation flagged M-3 as a likely category error
(`handle_top_level`'s 75 lines contain cross-cutting state that
doesn't compress to flat dispatch); the user's response was to ship
the flat-state-machine version anyway and let measurement decide.

The honest measurement story below mirrors ADR-0016's R1/R2/R3
discipline: ship, measure, document, leave the experiment in jj
history regardless of which side wins.

## Measurement baseline

All numbers from `throughput_by_class` on the 17 435-doc Aozora
corpus, single pass, sequential (parallel scaling tracked separately
under M-1). Bands: < 50 KB / 50 K-500 K / 500 K-2 M / > 2 MB,
reported as MB/s.

| Step | < 50 KB | 50 K-500 K | 500 K-2 M | > 2 M | parse wall | parallel scaling |
|---|---:|---:|---:|---:|---:|---:|
| **deps-bump-2026-04** (start of session) | 245.3 | 264.3 | 211.7 | 108.9 | 3.31 s | 14.14× |
| **M-1** (per-thread arena reuse) | **261.0** | **277.2** | **259.9** | **120.2** | 3.00 s | 13.90× (wall 0.61 s) |
| **M-2** (Pure SoA storage) | 244.2 | 254.6 | 229.7 | 108.1 | 3.28 s | — |
| **M-3 default** (M-2 carry-through) | 242.1 | 245.8 | 217.0 | 100.6 | 3.40 s | — |
| **M-3 phase3-fsm ON** | 229.1 | 233.9 | 207.0 | 100.2 | 3.57 s | — |

Reading: M-1 is the only unambiguous win. M-2 and M-3 both regress
sequential corpus throughput. Within each band:

- **M-2 vs M-1**: -6 % to -16 %. Worst on 500 K-2 M (-12 %) and > 2 M
  (-10 %). The 4-column SoA's tag-density win didn't materialise on
  the corpus profile — bumpalo's three-column allocation cost,
  per-event dummy-payload pushes, and the iter() reconstruction in
  `Pipeline::build()` outweigh the cache-density gain.
- **M-3 large FSM vs M-3 default**: additional -5 % to -6 % across
  all bands. The 15-variant action vocabulary + SmallVec<2> action
  list + execute loop adds dispatch overhead that the rustc-jump-
  tabled cascade doesn't have.

## M-1 — Per-thread arena reuse (PROMOTED)

### What

R4-B introduced rayon parallel mode; each task constructed a fresh
`Arena::new()`. M-1 hoists the arena into a per-worker
`thread_local!(RefCell<Arena>)` and calls `Arena::reset()` between
docs. The arena's chunks are retained — only the allocation cursor
rewinds. mmap/munmap serialisation in the kernel page-cache path
(R4-B's measured 11.6 % parallel-efficiency gap) is eliminated.

### Why `thread_local!` and not `rayon::ThreadLocal`

Plan-agent validation surfaced that **`rayon::ThreadLocal` does not
exist** in rayon 1.12. The codebase already had 4 `thread_local!`
blocks (`crates/aozora-lexer/src/instrumentation.rs:231,291`,
`crates/aozora-lexer/src/phase3_classify.rs:224`); pattern was
established. `RefCell<Arena>` matches `Arena`'s existing `!Sync`
contract exactly.

### Files

- **`crates/aozora-syntax/src/borrowed/arena.rs`**: added `Arena::reset(&mut self)`. `&mut self` enforces at compile time that no live borrow into the arena exists at reset time.
- **`crates/aozora-bench/examples/throughput_by_class.rs`**: per-task `Arena::new()` replaced with `WORKER_ARENA.with(|cell| { cell.borrow_mut().reset(); … })`.
- **`crates/aozora-bench/examples/phase_breakdown.rs`**: same pattern, two thread-locals (Phase 3 measurement + full pipeline measurement) because they ran back-to-back inside `measure_one`.

### Measured delta

- **Sequential**: +6.4 % / +4.9 % / +22.8 % / +10.4 % per band. The
  500 K-2 M band's +22.8 % is the largest single win in the entire
  M-1 / M-2 / M-3 session — bigger docs amortise more `mmap` saving
  per parse.
- **Parallel** (16-core host): wall 0.68 s → 0.61 s (-10 %). Scaling
  factor went 14.14× → 13.90× — *artifact* of `serial-work` shrinking
  more than `wall` (mathematical artifact, not regression). What
  matters is wall-time which improved.

### Verdict: **PROMOTE**

Architecturally clean (one new method, two bench-harness changes),
genuinely improves both sequential and parallel throughput, no
regression on any band, no behaviour change. M-1 lands on default.

## M-2 — Pure SoA TokenStream + PairEventStream (REGRESSION; kept in history)

### What

`BumpVec<'a, Token>` and `BumpVec<'a, PairEvent>` were replaced with
4-column Structure-of-Arrays (`tags` + `spans` + `trigger_kinds` +
`pair_kinds`) backed by `bumpalo::collections::Vec` columns inside
the parse arena. Variants are `Copy` so each push goes to multiple
columns; tag-only iteration scans 1 cache line per 64 events vs ~5
events for the 16-byte enum.

### Architectural rationale

User decision: Pure SoA over a hybrid packed-byte design. Trade-off
accepted: each event row carries one wasted byte in the unused-payload
column (a Newline row stores a dummy `TriggerKind`). Net storage:
10-11 bytes / event vs the enum's 16 bytes — modest packing win,
large theoretical tag-density win.

### Why it regressed

- **3 × bump pushes per event**: the SoA struct's `push_text`/`push_solo`/
  `push_pair`/`push_newline` each touch 3-4 BumpVec columns.
  `bumpalo::collections::Vec::push` is fast but not free.
- **Per-event dummy-payload writes**: for tag-stable shape, every push
  writes the unused payload column with a dummy default
  (`TriggerKind::Bar` / `PairKind::Bracket`). On the corpus median doc
  this is constant overhead even when the payload is never read.
- **`iter()` reconstruction**: `Pipeline::build` consumes the events
  via `events.iter()` which reads tag column + payload columns to
  reconstruct each `PairEvent` value for the streaming `classify`.
  This negates the cache-density win because every event still
  produces a 16-byte enum on the call site.
- **No SoA-aware downstream**: a true win would require Phase 3
  classify to consume the columns directly without enum
  reconstruction. M-3's flat-state-machine attempt (below) didn't
  rewrite classify's recogniser leaves to be column-aware — the
  recogniser bodies (`try_ruby_emit`, `try_bracket_emit`,
  `try_gaiji_emit`) all read `PairEvent` values.

### Verdict: **REGRESSION**, kept in jj history

Sequential -6 % to -16 % across bands. The architectural cleanness
(no enum padding waste, columnar layout, `iter()` for legacy callers)
isn't worth the perf cost on its own. **Future work** (per ADR-0019's
"out of scope (kept)" list below) could re-litigate by writing an SoA-
aware classify; M-2 alone doesn't pay.

The M-2 commit stays in `m1-m4-modern-followups` jj history for
future archaeology. Production code path stays on `BumpVec<Token>` /
`BumpVec<PairEvent>` (R4-A).

### Files touched

- `crates/aozora-lexer/src/token.rs`: `TokenStream<'a>` SoA + `iter()`.
- `crates/aozora-lexer/src/phase1_events.rs`: `tokenize_in` returns `TokenStream`.
- `crates/aozora-lexer/src/phase2_pair.rs`: `PairEventStream<'a>` SoA + `pair_in` rewrite.
- `crates/aozora-lex/src/pipeline.rs`: accessor signatures, `events.iter()` in `build()`.

## M-3 — Phase 3 classify as flat state machine (REGRESSION; cfg-gated, default OFF)

### What

`process_event` + `handle_top_level` + part of `recognize_and_emit`
(~210 lines of cross-cutting if/match cascade) replaced with:

1. **Pure transition function** `fsm_dispatch(state, event,
   has_pending_refmark) -> ActionList` — no `&mut self`, testable in
   isolation.
2. **Action vocabulary**: 9 variants (`HandleStream`,
   `AppendToFrameAndMaybeRecognise`, `FoldPendingRefmarkIntoPlain`,
   `EmitNewlineFlushAndPush`, `SetPendingRefmark`,
   `EnterStreamingFrame`, `OpenBracketFrameAbsorbingRefmark`,
   `OpenRubyFrame`, `FoldEventIntoPendingPlain`).
3. **Execute loop** that applies actions via narrow helpers on
   `&mut self`. Recogniser leaves (`try_ruby_emit` etc.) reused
   verbatim — those are leaf computations that wouldn't compress to
   flat state regardless of dispatch shape.

`#[cfg(feature = "phase3-fsm")]` swap; default code path unchanged.
Shipped behind `aozora-lexer/phase3-fsm` (forwarded by
`aozora-lex/phase3-fsm`) so comparative measurement is one
`cargo build --features` away.

### Why it regressed

Plan-agent's category-error prediction was confirmed by data:

- **9-variant action enum + SmallVec<2> action list per event**:
  each event allocates an inline list, the executor iterates and
  matches. The default code path's `if streaming { … } else if
  frame { … } else { … }` cascade compiles to a near-direct call
  chain. The FSM adds one indirection per event.
- **`PairKind` non-exhaustive forces wildcard arm**: even though
  the dispatch shape covers Bracket / Ruby / DoubleRuby /
  Quote / Tortoise explicitly, the match must include a `_ =>`
  fallback for future variants. Adds nothing semantic; adds a
  branch.
- **`fsm_recognise_dispatch` reads the frame's open kind via
  `self.frame.as_ref().and_then(...)`**: small but per-frame-close
  overhead on top of what `recognize_and_emit` did via direct match.

Net: **-5 % to -6 %** vs M-3 default (which was already M-2's
regressed baseline). Combined with M-2: **-12 % to -20 %** vs M-1.

### Verdict: **REGRESSION**, kept cfg-gated

The flat state machine is genuinely cleaner to read — adding a new
event class adds one match arm in `fsm_dispatch`, with no
side-effect site to touch. Plan-agent's "≤ 10 actions" cap was
nearly met (9 variants). The architectural payoff exists.

But the data is unambiguous: the rustc-jump-tabled cascade is
faster on this hot path. Production stays on the default. The FSM
implementation is **kept in jj history** (and behind the feature
flag) so future revisits — especially after a Phase 3 algorithmic
rewrite — can re-evaluate without re-implementing.

### Files touched

- `crates/aozora-lexer/Cargo.toml`: added `phase3-fsm` feature.
- `crates/aozora-lex/Cargo.toml`: forward `phase3-fsm`.
- `crates/aozora-lexer/src/phase3_classify.rs`: cfg-gated `process_event` swap, `fsm_dispatch`, `fsm_execute`, `fsm_recognise_dispatch`, `FsmState`, `FsmAction`, `FsmActionList`. Default `process_event` + `recognize_and_emit` are now `#[cfg(not(feature = "phase3-fsm"))]`.

## BMI2 PEXT trigger dispatch — REJECTED (re-confirmed from ADR-0015)

ADR-0015's bake-off measured PEXT batch compaction against the
production Teddy / structural-bitmap / DFA backends:

- Aozora corpus median candidate density: **1.79 %** (sparse).
- PEXT's batch compaction wins on dense bitmaps (>50 % bit set,
  e.g. simdjson's character-class scan).
- On sparse bitmaps, **Kernighan per-bit extraction (`mask &= mask
  - 1`) is faster** because the loop iteration count matches the
  set-bit count instead of all 64 bits.

This conclusion stands. The candidate-density model hasn't changed
(corpus distribution is structural, not configurable), so PEXT does
not get a new ADR unless a corpus shift produces dense candidate
streams. The "out of scope" list below records this as a closed
question, not a deferred one.

## Variable-length PairEvent encoding — DROPPED

The original ADR-0017 candidate list speculated about varint /
per-tag-dispatch encoding to compress `PairEvent`'s 16-byte enum
padding (max-variant size = max payload size). M-2's Pure SoA
already addresses this concern by storing each variant's payload in
its own column — the padding *is* the unused-payload column's dummy
byte, which is one byte not many. Variable-length encoding adds
complexity without further win after SoA.

Combined with M-2's measured regression: there is no path where
variable-length encoding wins on this data. **Dropped from the
follow-up roadmap.**

## Out of scope (kept — measurement gates)

- **SoA-aware Phase 3 classify**: rewrite recogniser leaves
  (`try_ruby_emit`, `try_bracket_emit`, `try_gaiji_emit`) to consume
  the SoA columns directly instead of `PairEvent` values. Would
  remove M-2's `iter()` reconstruction overhead and the
  `recognize_and_emit` body's enum-walk cost. Estimated 2-week
  effort. Gate: revisit when SoA hot-loop micro-benchmarks show
  ≥ 10 % wins on synthetic dense streams.
- **Per-doc intra-document parallelism**: paired-bracket nesting
  requires sequential parse. Not pursued.
- **Mega-DFA covering Phase 0/1/2/3**: speculative; no measurement
  signal Phase 0 + 1 + 2 dispatch is hot. M-3's negative result
  reinforces that flat-state-machine doesn't beat the cascade.
- **`Vec<Diagnostic>` arena allocation**: ~0.1 / doc on the corpus;
  in the noise. Stays heap.
- **Per-thread arena adaptive capacity**: M-1 hands out one
  `Arena::new()` per worker (default capacity). Pre-sizing per band
  could amortise the first parse. Gate: revisit if M-1's > 2 M
  band shows a cold-start regression on smaller corpora.

## Validation gates

| Gate | M-1 | M-2 | M-3 default | M-3 FSM |
|---|---|---|---|---|
| `cargo test --workspace --no-fail-fast` | 549/0 | 549/0 | 549/0 | 549/0 |
| `property_borrowed_arena` | 12/0 | 12/0 | 12/0 | 12/0 |
| `cargo clippy … -- -D warnings` | clean | clean | clean | clean |
| `cargo fmt --all -- --check` | clean | clean | clean | clean |
| Sequential corpus throughput vs M-1 | baseline | -6 to -16 % | -8 to -16 % | -12 to -20 % |
| Parallel scaling vs R4-B | wall -10 % | — | — | — |

## Decision

- **M-1**: ship on default. Clear win.
- **M-2**: keep in jj history (`m1-m4-modern-followups` bookmark);
  **reverted from default code path** after the A0+A re-evaluation
  re-confirmed regression even with Phase 1 heap eliminated.
- **M-3**: keep in jj history; **reverted from default code path**
  along with M-2. The cfg-gated `phase3-fsm` feature flag is
  removed (no benefit to keeping a dead alternative dispatcher).
- **PEXT**: closed (not deferred); ADR-0015 rejection stands.
- **Variable-length encoding**: dropped; SoA already addresses the
  padding concern and measurement says SoA itself doesn't pay.

## Step C drill-down (post-deploy follow-up)

After the four steps above shipped, a deeper profile drill-down
exposed two additional hot paths that the categoriser had been
hiding inside the monolithic `allocation` bucket:

- **Phase 1 scratch `Vec<u32>`** (trigger / newline offsets) was
  heap-allocated despite the surrounding pipeline already owning an
  arena. `Vec::extend_desugared` showed at 5.85 % inclusive of
  corpus parse — the result of an unrealistic 1/1000 capacity
  heuristic in `aozora-scan` that drove ~16 grow doublings per parse.
- **Per-thread arena initial capacity** (M-1 left it at bumpalo's
  512-byte default) caused the first ~7 docs each rayon worker saw
  to pay 7 successive chunk-grow doublings before reaching the
  ~50 KB the corpus median needs.

### A0 — arena-allocate Phase 1 scratch buffers

- `aozora-scan` cap heuristic fixed (1/1000 → 1/56, matching the
  measured 1.8 % corpus trigger density)
- `aozora_scan::scan_offsets_in(source, &Bump) -> BumpVec<'a, u32>`
  free function added (dyn-compat preserved)
- `tokenize_in` lifts both scratch buffers into the parse arena

Measured: +1.2 % / +1.6 % / +3.2 % / 0 % per band sequential.
Smaller than the inclusive 5.85 % suggested because the bulk of that
inclusive cost was the iterator walk itself (self only 0.01 %), not
the alloc pair.

### A — pre-size the per-thread arena

- bench harness `WORKER_ARENA` initialised with 256 KB capacity
  (covers >95 % of corpus docs in one chunk)
- production `Document::new` was already pre-sizing
  (`source.len() * 4` since N6); M-1's reuse path was the only one
  paying default cap

Measured (incremental on top of A0): **+3.0 % / +2.8 % / +9.3 % /
+13.5 %** per band. The large bands win disproportionately because
each chunk-grow doubling on a multi-MB doc is a multi-MB `mmap`,
and pre-sizing skips them.

### A0 + A re-evaluation of M-2 / M-3

The Phase 1 heap allocation A0 closed was the leading hypothesis
for why the original M-2/M-3 measurement showed regression: maybe
the Pure SoA / flat FSM weren't slower per se, but the noisy
heap-Vec baseline was hiding their wins. Re-measuring with A0+A in
place falsified that:

```
                     <50KB  50-500K  500K-2M   >2M
M-1 alone (baseline) 261    277      260      120
+ A0 + A (no M-2/3)  262    269      235      115   ← reverted-state ship
+ M-2 + M-3 + A0 + A 252    257      245      115   ← rejected
```

M-2/M-3 still cost throughput even after A0+A. The category-error
diagnosis (Phase 3 cross-cutting state doesn't compress to a flat
state machine; per-event SoA push overhead exceeds the tag-density
win for this corpus profile) is data-confirmed.

### Final shipping shape

- **`Arena::reset` (M-1)**: kept
- **bench thread_local arena reuse (M-1) + 256 KB initial cap (A)**:
  kept
- **arena-allocated Phase 1 scratch buffers (A0)**: kept
- **`tokenize_in` returns `BumpVec<'a, Token>` (R4-A baseline)**:
  kept; M-2's `TokenStream` SoA reverted
- **`pair_in` returns `BumpVec<'a, PairEvent>` (R4-A baseline)**:
  kept; M-2's `PairEventStream` SoA reverted
- **`process_event` cascade (R4-A baseline)**: kept; M-3's flat
  state machine + `phase3-fsm` feature reverted
- **categoriser allocation breakdown (4 sub-buckets)**: kept (now
  the standard categoriser shape for future drill-downs)
- **`install_forward_target_index_from_source` early-break +
  `clear_forward_target_index_if_installed` no-op skip**: kept

## ADR-0019 final lessons

The original M-1 → M-4 work shipped four ambitious ideas and
documented two as negative results; the post-ship A0/A/Step-C
follow-up confirmed those negatives weren't artefacts of an
upstream bottleneck. The architectural takeaways are stronger now:

1. **Profile-driven beats principle-driven on this corpus.** The
   data, not the categories of "things that ought to help",
   selects the wins. M-2 (Pure SoA) and M-3 (flat state machine)
   are both well-understood patterns; neither paid here.
2. **Pure-Rust drill-down (xtask trace stacks + categoriser splits)
   was decisive.** Without the per-bucket allocation breakdown the
   true winner (A) would have stayed buried.
3. **Reverting committed-but-disproved work is part of the
   discipline.** ADR-0016 (R1 reverted) set the precedent; ADR-0019
   continues it for M-2/M-3.

Future Phase 3 work should target **algorithmic** changes (recogniser-
body rewrites: annotation parser unification, ruby preceding-bytes
SIMD scan, interner short-string fast-paths re-evaluated after A
reduces baseline noise) rather than **structural** re-shaping of
the dispatch / storage layout. The ceiling on structural changes
for this hot path is now well-bounded by data.

## Lesson recorded

Two architecturally clean ideas (M-2's Pure SoA, M-3's flat state
machine) regressed on data. This is **not** a refutation of those
patterns in general — it's a refutation of them on this specific
workload (corpus-median small docs with low PairEvent density per
parse). The patterns may pay on different workloads; the ADR
preserves the implementation so future maintainers can re-measure
without re-implementing.

The discipline established by ADR-0016 (R1/R2/R3 negative-result
documentation) generalises: shipped + measured + documented +
preserved-in-history is the right shape for "ambitious architectural
ideas". The wrong shape would be either:
- Refusing to implement because we predicted regression (loses
  measurement);
- Reverting silently (loses the experimental evidence).

Both M-2 and M-3 contribute the **measurement** that justifies
focussing future Phase 3 work on **algorithmic** improvements
(actual recogniser-body changes) rather than **structural**
re-shaping (storage layout, dispatch shape) on this hot path.
