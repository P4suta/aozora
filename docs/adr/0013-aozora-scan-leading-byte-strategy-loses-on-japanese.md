# 0013. aozora-scan's leading-byte strategy is not faster than legacy phase 1 on Japanese text

- Status: accepted (negative result)
- Date: 2026-04-26
- Deciders: @P4suta
- Tags: architecture, performance, lex, simd, 0.2.0

## Context

ADR-0009 plans the `aozora-scan` crate as the SIMD substrate that
replaces phase 1's character-by-character walker. ADR-0012 § I-1
pinned the strategy as **simdjson-style**: a SIMD scan finds
trigger-leading-byte candidates, then a const-PHF lookup performs
precise classification on each candidate.

Move 4 shipped aozora-scan with three backends: scalar (memchr3),
AVX2, and NEON / wasm-simd scaffolds. The AVX2 backend measured
1.5-1.6× faster than the scalar baseline in the standalone scan
benchmark.

The fused-engine work then attempted to replace phase 1 of the lex
pipeline with `aozora_lex::tokenize_with_scan`, an offset-driven
walker built on `aozora_scan::best_scanner`. The byte-identical
proptest passed against random aozora-shaped input. The corpus
benchmark **regressed by ~30%** (117 MB/s → 76 MB/s), and a
follow-up criterion bench isolated the cause to the new tokenizer
itself:

| input band (64 KiB) | legacy phase 1 tokenize | scan-driven tokenize | ratio       |
|---------------------|-------------------------|----------------------|-------------|
| plain Japanese      | 77 µs                   | 416 µs               | 5.4× SLOWER |
| sparse triggers     | 80 µs                   | 426 µs               | 5.3× SLOWER |
| dense triggers      | 93 µs                   | 363 µs               | 3.9× SLOWER |

## Decision

Revert phase 1 in `aozora_lex::engine::run_pipeline` to the legacy
character-walking [`aozora_lexer::tokenize`]. Keep
`aozora_lex::tokenize_with_scan` as a maintained but unused
artifact: its byte-identical unit tests, its 18-anchor +
3-strategy proptest pin, and the
`benches/tokenize_compare.rs` criterion bench provide the
regression substrate a future fused-engine redesign starts from.

The aozora-scan crate itself stays in place. It still wins as a
**standalone scanner** for ASCII-heavy inputs (where the
trigger-leading bytes are genuinely sparse) and as a building
block for the multi-target driver crates (Move 4). Its trait-based
backend dispatch is the right shape; only the in-lex-pipeline
integration strategy needs rethinking.

## Why the leading-byte strategy loses on Japanese

`aozora-scan` searches for the trigger-leading-byte set
`{0xE2, 0xE3, 0xEF}` — every full Aozora trigger character begins
with one of these three bytes. The strategy assumes those bytes
are **rare** in non-trigger source: the SIMD scan zooms over plain
text at memory-bandwidth speed, then per-candidate classify pays
only for the rare hits.

That assumption holds for ASCII (where 0xE3 is invalid as a
stand-alone byte) and even for Latin-1-heavy text. **It fails
catastrophically for Japanese**:

- Every hiragana character in `U+3041..U+30FF` begins with `0xE3`
  in UTF-8.
- Every katakana character in `U+30A0..U+30FF` likewise.
- Many CJK ideographs in `U+3400..U+9FFF` also begin with `0xE3`.

A typical paragraph of Japanese prose is therefore *almost
entirely* leading-byte candidates, and `memchr3` returns a
candidate roughly every 3 source bytes. The per-candidate PHF
classify rejects 99%+ of these candidates, but at that point we
have done the same per-character work as the legacy walker —
plus the overhead of materialising candidate offsets in a
`Vec<u32>` and merge-walking with newline offsets.

## Consequences

**Honest baseline preserved**:
- The corpus profile remains at the post-fat-LTO baseline (117
  MB/s per-thread). No regression shipped.
- The 18 byte-identical proptest anchors still validate phase 1's
  correctness, now against the legacy walker (the original baseline).

**The fused engine plan needs a different strategy** for phase 1:

1. **simdjson-style structural bitmap with full 3-byte compare.**
   Build a bitmap over the source bytes where bit `i` is set iff
   the 3-byte window at byte `i` is one of the eleven full trigger
   sequences. AVX2 can do this with three `_mm256_cmpeq_epi8`
   layered passes (one per byte of the trigger). Lossier than the
   leading-byte filter but provably zero false positives. Estimated
   2-3× speedup vs legacy on Japanese.

2. **DFA-based tokenizer.** Build a small DFA over UTF-8 bytes
   recognising every trigger as a path. Walk source bytes through
   the DFA in a tight loop. Branch-predictor-friendly and avoids
   the per-character UTF-8 decode the legacy walker still pays.

3. **Hybrid: keep the legacy walker, add memchr3 as a
   fast-forward within all-ASCII runs only.** When the loop is in
   an ASCII range it can `memchr3` to the next non-ASCII byte;
   when it's in a multi-byte run it walks character-by-character.
   This gives the simdjson win on ASCII source without losing on
   Japanese source.

The next fused-engine commit will pick one of these and re-attempt
the integration, with the criterion bench as the gate.

## Alternatives considered

- **Ship the regression and recover later** (30% slowdown
  guaranteed). Rejected: the "byte-identical AND faster" contract
  is the whole reason to swap.
- **Keep tokenize_with_scan + the legacy tokenizer side-by-side
  with a runtime selector based on input language detection**.
  Adds runtime cost on every parse to detect the language, and
  the predicate is tricky (mixed Japanese/English text is common
  in editorial annotations). Rejected for complexity.
- **Drop aozora-scan entirely from the lex path; keep it only
  for the standalone driver crates** (e.g., a corpus indexer that
  only wants trigger positions, not tokens). Reasonable but
  premature — the structural bitmap approach above can still rescue
  the in-pipeline integration.

## References

- ADR-0009 (Clean layered architecture) — fused engine plan
- ADR-0012 (Algorithmic baseline) — pinned aozora-scan as I-1
- `crates/aozora-lex/src/tokenize.rs` — the maintained-but-unused
  `tokenize_with_scan` implementation + tests
- `crates/aozora-lex/benches/tokenize_compare.rs` — the criterion
  bench that produced the table above
- Lemire & Langdale, "Parsing gigabytes of JSON per second"
  (2019) — the structural-bitmap reference; `aozora-scan` as
  currently implemented does **not** yet realise this technique.
- Daniel Lemire's blog post "memchr (the fastest function?) and
  the impact of leading-byte distribution" — the underlying
  observation that motivated revisiting the strategy choice.
