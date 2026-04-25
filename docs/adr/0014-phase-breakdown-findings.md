# 0014. Phase breakdown findings — measured 2026-04-26

- Status: accepted (measurement record)
- Date: 2026-04-26
- Deciders: @P4suta
- Tags: performance, lex, profiling, 0.2.0

## Context

Earlier optimisation discussions estimated per-phase costs by
inspection: phase 1 tokenize "probably ~14%", phase 3 classify
"probably 30-40%", phase 4 normalize "probably 20-30%". The
fused-engine attempt that targeted phase 1 (ADR-0013) shipped a
5.4× regression because the underlying assumption — that
`memchr3`-based scan would beat per-character classify on Japanese
text — was wrong.

To stop guessing, this commit adds
`crates/aozora-bench/examples/phase_breakdown.rs` which times each
of the six phase functions individually across the full Aozora
corpus and reports per-phase wall-clock and per-doc latency
quantiles. This ADR records the findings and revises the
optimisation roadmap accordingly.

## Measurement

Run on the full 17,435-doc corpus (791.51 MB sanitised UTF-8) on
the dev machine (Ryzen 7 5800H, fat LTO release build,
2026-04-26). All six phases called individually with `Instant`
timing around each call.

### Per-phase totals

| phase | wall ms | % of parse total | per-thread MB/s |
|-------|---------|------------------|-----------------|
| 0 sanitize  | 1362 |  **23.2%** |   581 |
| 1 tokenize  | 1087 |  **18.5%** |   728 |
| 2 pair      |  133 |    2.3% | 5,971 |
| 3 classify  | 2230 |  **38.0%** |   355 |
| 4 normalize |  416 |    7.1% | 1,903 |
| 6 validate  |  648 |  **11.0%** | 1,222 |
| ALL         | 5876 |  100.0% |   135 |

### Pathological docs (top-3 by classify cost)

| doc | bytes | classify ms | total ms | classify % |
|-----|-------|-------------|----------|------------|
| #5667 (`明治人物月旦`) | 731 KB | 170.92 | 173.70 | **98.4%** |
| #686 (Dostoevsky's longer translations) | 1.39 MB | 83.65 | 91.64 | 91.3% |
| #14786 | 1.00 MB | 44.47 | 49.29 | 90.2% |

### Per-doc latency quantiles (microseconds, full corpus)

| phase | p50 | p90 | p99 | max | mean |
|-------|-----|-----|-----|-----|------|
| sanitize  |  26.8 |  137.0 |   959 | 11.6 ms | 78.1 |
| tokenize  |  21.3 |  113.6 |   775 | 13.5 ms | 62.3 |
| pair      |   1.7 |   10.9 |    82 | 11.3 ms |  7.6 |
| classify  |   6.8 |   83.5 | 2,207 | **170.9 ms** | 127.9 |
| normalize |   4.7 |   36.9 |   271 | 20.5 ms | 23.9 |
| validate  |  12.2 |   67.2 |   467 |  3.9 ms | 37.2 |
| TOTAL     |  78.9 |  448.9 | 4,766 | 173.7 ms | 337.0 |

## Findings

### Surprises vs prior estimates

| phase | estimated | measured | delta |
|-------|-----------|----------|-------|
| 0 sanitize | ~5% | **23%** | 4.6× higher than expected |
| 4 normalize | 20-30% | **7%** | 3-4× lower than expected |
| 6 validate | ~5% | **11%** | 2× higher than expected |

The estimate that drove the original "Innovation I-4 (bumpalo
arena adoption)" ROI of 10-20% was based on phase 4 being
20-30% of total. **The measured phase 4 is only 7%.** Even
halving it nets only +3% total; halving the per-Box allocation
overhead realistically saves 2-3% on average.

### Phase 3 classify dominates, but bimodally

Average parse spends 38% of wall-clock in classify, but the
distribution is heavily skewed:

- p50: 6.8 µs (basically free)
- p99: 2.2 ms (200× p50)
- max: **170.9 ms** (1 doc)

The pathological doc (`明治人物月旦`, 252 reps of
`※［＃白ゴマ、1-3-29］`) spends 98% of its parse time in classify.
This isn't a small constant overhead — it's a **per-annotation
linear scan** through the prefix-recognition logic. Aho-Corasick
collapses that linear scan into a single O(n) DFA traversal.

### Phase 0 sanitize is unexpectedly hot

23% wall-clock for `sanitize` is suspicious. The phase does:

1. BOM strip (ASCII test, near-free)
2. CRLF → LF normalisation (one `contains('\r')` check + maybe rewrite)
3. Decorative-rule isolation (one `contains` check + maybe rewrite)
4. `〔...〕` accent decomposition rewrite
5. PUA collision scan (full text walk via `chars()`)

The PUA collision scan walks every character once via a `matches!`
predicate. That's redundant with phase 1's tokenize walk and could
be folded into phase 1. The accent-decompose rewrite only fires when
`〔` is present (rare on this corpus); it's not the culprit.

**Likely culprit: the per-character PUA scan in phase 0.** Folding
it into phase 1 (or skipping it entirely in release builds when the
diagnostic isn't surfaced) could save 5-10% total.

### Phase 6 validate is skippable in release

11% wall-clock for invariants V1-V3 (residual `［＃` in normalised
text, unregistered PUA sentinels, registry sort + position match).
These are correctness checks the lexer's own author wrote to
catch lex pipeline bugs, not user-input issues. Production builds
that have already exercised the corpus sweep don't need them.

Adding `#[cfg(debug_assertions)]` (or a `validate-invariants` cargo
feature) to `phase6_validate::validate` would recover 11% near-free.

## Revised optimisation roadmap

The original "C → B" plan (Aho-Corasick → bumpalo arena) needs
revision based on the data:

### Tier 1 — 確実な大物 (do these first)

1. **Phase 3: Aho-Corasick annotation classifier (Innovation I-6).**
   - Average win: 5-15% (classify is 38% of average; AC cuts
     annotation prefix recognition from O(n×m) to O(n))
   - Pathological win: **massive** (170ms → estimated <10ms for
     `明治人物月旦`)
   - Target ADR section: pin annotation prefix list, build
     compile-time DFA via `aho-corasick` crate, cross-check
     byte-identical via proptest
   - Implementation effort: 1 session

2. **Phase 6: skip validate in release.**
   - Win: 11% near-free
   - Implementation effort: ~30 minutes (gate behind cfg or feature)
   - Risk: production silently runs without the V1-V3 sanity gate.
     Mitigated by keeping it on in test + corpus-sweep CI.

### Tier 2 — 中物 (worth doing after Tier 1)

3. **Phase 0: fold PUA collision scan into phase 1.**
   - Win: 5-10% (sanitize drops from 23% to ~10-15%)
   - Implementation effort: ~1 hour. Phase 1's char walk already
     iterates every byte; adding a `matches!` predicate inline is
     trivial.
   - Risk: change in diagnostic ordering. Mitigated by the
     byte-identical proptest.

### Tier 3 — 小物 + アーキ整理 (do when fundamentals settle)

4. **Phase 4: bumpalo arena adoption (Innovation I-4).**
   - Revised win estimate: **5-10%** (phase 4 is only 7% total).
     Below the 10-20% the plan claimed.
   - Architectural value: realises the unused
     `aozora-syntax::borrowed` module and unblocks the visitor
     renderer (Innovation I-10).
   - Implementation effort: 1-2 sessions
   - Recommendation: do for architectural reasons, not perf.

5. **Phase 1: revisit fused tokenize.**
   - Now known: legacy tokenize is 18% and tight. ADR-0013
     established that a leading-byte SIMD scan loses on Japanese.
     Either go full simdjson-style 3-byte structural bitmap (high
     complexity for marginal win) or leave it.
   - Recommendation: leave it until a new approach has a measurable
     edge in microbench.

## Decision

Adopt the Tier 1 / Tier 2 / Tier 3 stack above. Next commits, in
order: Aho-Corasick (1) → release-validate skip (2) → phase 0 fold
(3). Tier 3 work after that, gated on the ROI staying realistic.

## References

- `crates/aozora-bench/examples/phase_breakdown.rs` — the
  measurement harness produced for this ADR
- `crates/aozora-bench/benches/crime_and_punishment.rs` — single-
  doc bench used to anchor per-parse latency
- ADR-0013 — the falsified-fused-tokenize negative result that
  motivated this measurement
- ADR-0012 — the original innovation roadmap; this ADR's findings
  revise its I-1 / I-4 / I-6 ROI estimates
