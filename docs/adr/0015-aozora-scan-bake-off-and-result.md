# 0015. aozora-scan v2 — published-algorithm bake-off, Teddy wins

- Status: accepted
- Date: 2026-04-27
- Deciders: @P4suta
- Tags: architecture, performance, lex, simd, 0.2.0
- Supersedes: ADR-0013

## Context

ADR-0013 shipped the negative-result of T1: the v1 `aozora-scan`
(memchr3 over trigger leading bytes `{0xE2, 0xE3, 0xEF}` + handwritten
AVX2 over the same set) was 3.7× *slower* than the legacy phase 1
char-walker on Japanese-heavy text. Root cause: 0xE3 is the leading
UTF-8 byte of every hiragana / katakana / many CJK ideographs, so
the candidate filter saturated at ~33 % of source bytes — destroying
the candidate-sparsity the design assumed.

ADR-0013 enumerated three follow-up strategies but did not pick one.
The user re-framed this as a green-field rebuild opportunity: rather
than picking the obvious next step, *enumerate published modern
techniques as competing implementations and let the bench choose*.

## Decision

Replace the v1 leading-byte scan with a four-backend bake-off behind
the existing `TriggerScanner` trait. After measurement (see below),
**Teddy** (Hyperscan multi-pattern fingerprint matcher, exposed via
`aho_corasick::packed::Searcher`) is the production winner.
Dispatcher order:

1. **Teddy** — when SSSE3 is available (every `x86_64-v2`+ host).
2. **Structural-bitmap** — `x86_64` + AVX2 fallback (simdjson-style
   two-byte filter built from `_mm256_cmpeq_epi8`).
3. **Multi-pattern DFA** — universal SIMD-free fallback via
   `regex_automata::dfa::dense::DFA::new_many`.
4. **Naive** (`#[doc(hidden)]`, test-only) — brute-force PHF
   reference; cross-validates the other three via proptest.

The v1 leading-byte `ScalarScanner` and `Avx2Scanner` are retained
for now as bake-off targets (regression sentinels) but are no longer
selectable by `best_scanner()`. A follow-up CL will delete them
once the user authorises.

## The four published techniques considered

| # | Name | Algorithm sketch | Source |
|---|---|---|---|
| 1 | **Teddy** | PSHUFB shuffle-lookup tables hash 3-byte fingerprints to a per-pattern bucket bitmap; AND across positions; verify candidates | Geoff Langdale, Hyperscan (Intel, 2015); BurntSushi `aho-corasick` port (2019) |
| 2 | **Structural bitmap** (AVX2) | Per-chunk `_mm256_cmpeq_epi8` builds a 32-bit candidate bitmap from a two-byte (lead × middle) filter; Kernighan-iterate set bits; PHF verify | Langdale & Lemire, simdjson (VLDBJ 2019) |
| 3 | **Multi-pattern DFA** | Hoehrmann-style dense byte DFA where 11 trigger trigrams are accepting paths | Hoehrmann 2010; generalised by `regex-automata` |
| 4 | **Naive** (ground truth) | Brute-force position-by-position 3-byte window through `classify_trigger_bytes` | — |

PEXT-based bitmap compaction (`_pext_u64`, the canonical simdjson
trick for dense bitmaps) was evaluated and rejected for *this*
workload: at the corpus-measured 1.79 % trigger density the
candidate bitmap is too sparse for PEXT batch extraction to beat
per-bit Kernighan iteration. Kept as a future profiling-driven
escape hatch.

## Bake-off measurements

64 KiB synthetic input, criterion `--quick`, single host (Intel
x86_64, AVX2 + BMI2 + SSSE3 available):

| Backend | plain_japanese | sparse_triggers | dense_triggers | ascii_text |
|---|---|---|---|---|
| naive               |   78 MiB/s |   79 MiB/s |   79 MiB/s |   79 MiB/s |
| scalar (v1, memchr3 leading-byte) |  108 MiB/s |  110 MiB/s |  277 MiB/s |  **66 GiB/s** |
| avx2 (v1, leading-byte SIMD)      |  159 MiB/s |  160 MiB/s |  432 MiB/s |   36 GiB/s |
| structural_bitmap (v2, lead × mid filter) | **19.5 GiB/s** |  8.8 GiB/s |  418 MiB/s |   24 GiB/s |
| **teddy** (v2)      | **19.4 GiB/s** | **10.8 GiB/s** | **776 MiB/s** |   19 GiB/s |
| dfa (v2 baseline)   |  391 MiB/s |  372 MiB/s |  142 MiB/s |  1.8 GiB/s |

**Teddy wins or ties** on every realistic workload band:

- `plain_japanese`: 19.4 GiB/s vs v1's 159 MiB/s = **125× faster**
- `sparse_triggers` (corpus median): 10.8 GiB/s vs v1's 160 MiB/s = **67× faster**
- `dense_triggers`: 776 MiB/s vs v1's 432 MiB/s = **1.8× faster**

The v1 backends only win on `ascii_text` (where memchr3 has
near-zero candidates and runs at literal memory-bandwidth). That
workload doesn't apply to the actual Aozora corpus, so the v1 niche
disappears in production.

## Empirical evidence (T2.0 byte distribution)

Run on 1 999 Aozora corpus docs (~107 MB):

| Byte set | Density on corpus |
|---|---|
| Leading-byte set `{0xE2, 0xE3, 0xEF}` | 23.8 % of bytes (E3 alone: 23.2 %) |
| Middle-byte set `{0x80, 0xBC, 0xBD}` |  6.05 % of bytes |
| True triggers | 1.79 % of bytes |

Speedup ratio in the candidate-finding stage alone: middle-byte is
~3.93× sparser than leading-byte, but Teddy's 3-byte fingerprint
compaction is even more selective — measured 67× end-to-end on
`sparse_triggers`. The bench data (above) bears out the order-of-
magnitude difference between fingerprint-bucketed multi-pattern
matching and any single-byte filter.

## Production wire-up impact

`aozora_lexer::tokenize` (the canonical Phase 1 entry) now uses
`aozora_scan::best_scanner()`. End-to-end measurements:

- **Corpus throughput** (`throughput_by_class`): 248-274 MB/s by
  band — matches the post-N3 baseline (~254 MB/s mean), no
  regression on the corpus average.
- **Pathological doc 49178** (the doc T1 regressed on): tokenize
  phase moved from legacy 0.41 ms → T2 0.60 ms. The SIMD scan
  itself takes only ~22 µs at Teddy's 10 GiB/s rate; the rest is
  per-event overhead in the merge-walk + Token construction. The
  legacy walker integrated all of that into a single tight byte loop
  with low per-byte cost on Japanese.

So the SIMD scan is dramatically faster than v1 (proven), but the
production tokenizer wraps it in a Token-construction pipeline that
eats most of the win on dense-trigger outliers. Net: corpus-flat,
pathological-doc 1.5× regression, but a clean, modern, published-
algorithm substrate to build on. Subsequent CLs can target the
merge-walk per-event overhead now that the scan itself is no longer
the bottleneck.

## Architecture notes

- **`Vec<u32>` output retained.** Considered alternatives (1-bit
  bitmap, `SmallVec`, streaming `Iterator`); the user-facing
  `TriggerScanner` trait eagerly returns a `Vec<u32>` of trigger
  start offsets. At ~2 % corpus trigger density, a bitmap would be
  8× larger than the `Vec<u32>`; SmallVec inline buffers can't
  cover the 7 k+ triggers in worst-case docs; streaming hurts the
  AVX2 chunk loop's structure. Eager Vec is the right shape.
- **`OnceLock<Searcher>` caching.** Both Teddy (PSHUFB table
  population) and DFA (NFA → DFA minimisation) have non-trivial
  build cost. Both are `Send + Sync` after build, so a
  `OnceLock<TeddyScanner>` / `OnceLock<DfaScanner>` static in
  `best_scanner` builds once per process.
- **`no_std` strategy.** Core trait + `NaiveScanner` stay
  `no_std`. Teddy + DFA gated behind `feature = "std"` (default-on)
  because their underlying crates need `std`.

## Cross-validation

Every backend is proptest-checked against `NaiveScanner` (256
random aozora-shaped inputs per run). The ground-truth reference
shares no constants or SIMD shape with the clever backends, so it
catches the failure mode where two SIMD backends silently agree on
the wrong answer.

## Alternatives considered

- **Middle-byte memchr3 swap** (rejected for being too obvious).
  Theoretical 3.93× speedup over leading-byte, but Teddy's
  3-byte fingerprint compaction is an order of magnitude better.
  Discarded after the bench data made the gap visible.
- **Vectorscan FFI bindings** (the actively-maintained Hyperscan
  fork). Industrial-grade but adds a 2 MB binary cost and FFI
  surface for 11 patterns. Not justified.
- **Bit-parallel Shift-And NFA** (Baeza-Yates 1992). Scalar-only
  ~1-3 GB/s; loses to all three SIMD candidates by construction.
  Skipped from the bake-off.
- **Custom AVX2 PSHUFB shuffle-classification** (reinvented Teddy's
  N=1 case). Building the same algorithm from scratch when
  `aho-corasick::packed` already ships a hand-tuned production
  implementation would be redundant.

## References

- ADR-0009 (Clean layered architecture) — fused engine plan
- ADR-0012 (Algorithmic baseline) — pinned aozora-scan as I-1
- ADR-0013 (predecessor, superseded by this ADR)
- Langdale & Lemire, "Parsing Gigabytes of JSON per Second"
  (VLDB Journal 2019) — structural-bitmap technique
- Geoff Langdale, "Teddy: A literal matcher for short patterns"
  (Hyperscan internals, 2015)
- BurntSushi, `aho-corasick` `packed::Searcher` source +
  `teddy/README.md` (2019)
- `crates/aozora-bench/examples/byte_distribution.rs` — empirical
  byte-density probe (the T2.0 measurement)
- `crates/aozora-scan/benches/scanner_bakeoff.rs` — the bake-off
  bench that produced the table above
