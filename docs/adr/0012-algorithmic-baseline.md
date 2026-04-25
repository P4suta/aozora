# 0012. Algorithmic baseline: simdjson scan, Eytzinger search, Aho-Corasick

- Status: accepted
- Date: 2026-04-26
- Deciders: @P4suta
- Tags: architecture, algorithms, performance, 0.2.0

## Context

The 0.2.0 plan calls out twelve algorithmic innovations
("I-1 through I-12") spread across the parser. Several arrived
opportunistically as the layered crate split landed (Move 1
through Move 4). This ADR pins the **canonical algorithm choice**
for each load-bearing innovation so future commits don't silently
swap them out, and so a downstream consumer can reason about
expected performance characteristics without reading the source.

The performance targets they collectively underwrite are:

| metric                                | 0.1 baseline | 0.2.0 ceiling |
|---------------------------------------|--------------|---------------|
| 罪と罰 (2 MB) per-core parse P50      | 20 ms        | ≤ 5 ms        |
| per-core throughput                   | 96 MB/s      | ≥ 400 MB/s    |
| 16-thread aggregate                   | 627 MB/s     | ≥ 4 GB/s      |
| AST tree memory for 罪と罰            | ~100 KB      | ≤ 30 KB       |
| WASM browser parse 罪と罰             | n/a          | ≤ 100 ms      |

Some innovations land in the lex layer (Move 2 follow-up) and some
already shipped through Move 1's syntax/registry redesign.

## Decision

Pin the following algorithm choices per innovation. The proptest
+ corpus-sweep harness that ships with each innovation is the
contract that keeps the choice honest.

### Shipped in Move 1-4

**I-4 — Bumpalo arena per Document.** `bumpalo::Bump` is the
arena. Comrak (a transitive workspace dep already) uses bumpalo;
adding it costs no new dependency. The borrowed AST in
`aozora-syntax::borrowed` is built to allocate into one Bump per
parse and drop the entire arena as a unit, eliminating per-node
`Box::new` malloc calls (Move 1.4).

**I-5 — SoA registry + cache-oblivious search.** Registry is
laid out as struct-of-arrays (`EytzingerArray<u32>` for keys +
parallel `Vec<V>` for payloads). The Eytzinger key layout is the
substrate from `aozora-veb` — cache-friendlier than
`Vec::binary_search` at sizes ≥ L1 (~16k entries). A vEB
implementation can later swap in behind the same trait if a
benchmark warrants (Move 1.3, Move 1.4).

**I-7 — String interning + arena-borrowed text.** `Content::Plain`
is `&'src str` rather than `Box<str>` in the borrowed AST.
Arena-backed string interning (deduplicating repeated readings
like `《の》`, `《に》`) is **deferred** until Move 2's fused
engine actually exercises the interner; the Arena currently
exposes `alloc_str` only.

**I-9 — Const-compiled trigger PHF.** `aozora-spec`'s
`SINGLE_TRIGGER_TABLE` is a `phf::Map<[u8; 3], TriggerKind>`
covering the eleven single-character triggers. Lookup is a
branch-free O(1) PHF probe rather than a `match` chain;
exercised by every SIMD scanner backend after the candidate-byte
filter (Move 1.1).

**I-1 — simdjson-style structural scan (partial).** `aozora-scan`
ships with three backends today:
- `ScalarScanner` — `memchr::memchr3` over the
  `{0xE2, 0xE3, 0xEF}` leading-byte set. memchr already vectorises
  internally on x86_64 + aarch64.
- `Avx2Scanner` — `x86_64` AVX2: 32-byte `_mm256_cmpeq_epi8`
  loads → `_mm256_movemask_epi8` projection → trailing-zeros loop
  → const-PHF precise classify. **Measured 1.5-1.6× faster than
  scalar across plain / sparse / dense corpus bands** (criterion
  bench, 2026-04-26).
- `NeonScanner` / `WasmSimdScanner` — scaffolds; full
  implementations land when aarch64 / wasm32 dev hosts are
  configured.

The simdjson "structural bitmap + BMI2 PEXT" pattern is
**not yet implemented** — at current candidate density (< 1
trigger per 32-byte chunk on average) the trailing-zeros loop is
already at the lower bound of what the precise classify needs.
PEXT becomes the right move once aozora-lex's fused engine fuses
the candidate scan and classify into a single pass over the
arena (Move 2 follow-up).

### Pending follow-ups

**I-2 — Iterator-fusion deforestation.** The legacy 7-phase
pipeline still runs four materialised Vecs (`Vec<Token>`,
`Vec<PairEvent>`, `Vec<ClassifiedSpan>`, `String + Registry`).
Move 2's fused engine collapses this into a single iterator chain
that lands directly into the bumpalo arena. Defer this until
proptest coverage of the existing phase boundaries is complete
enough to detect a regression — currently 729 tests across the
workspace plus 18 byte-identical proptests in
`aozora-lex/tests/property_byte_identical.rs`.

**I-3 — Type-state pipeline.** New stage types (`Raw<'src>`,
`Scanned<'src>`, `Classified<'src>`, `Built<'src>`) gate phase
ordering at compile time. Lands inside Move 2's fused engine
because the type-state types only make sense as the public
boundary of the new lex layer.

**I-6 — Aho-Corasick annotation classifier.** Compile-time
`aho-corasick::DFA` covering ~30 annotation prefixes
(`ここから字下げ`, `ここから割書`, `罫囲み`, …). The pathological
doc identified in the 2026-04-25 corpus profile (鳥谷部春汀
『明治人物月旦』, 252 reps of `※［＃白ゴマ、1-3-29］`, 232 ns/byte
parse cost) collapses to scalar speed once Aho-Corasick is in
place. Lands in Move 2's fused engine.

**I-10 — Visitor-pattern renderer.** `aozora-render` ships today
as a thin façade re-exporting `aozora-parser`'s monolithic HTML
renderer. The visitor surface (`trait AozoraVisitor<'src>` with
default-impl walks) lands once Move 2's fused engine produces
borrowed AST natively, so the visitor walks `&'src AozoraNode<'src>`
without re-parsing source.

**I-12 — SJIS-aware decode fusion.** Decoder + scanner combined
into a single byte-stream pass. Optional, gated on a profile
showing the SJIS decode step (currently ~30% of corpus-sweep
wall-time) is worth the implementation cost.

### Out of scope

**I-11 — Persistent CST with structural sharing (HAMT).**
Reserved for a 0.3.0 incremental-LSP pivot. Not needed for the
0.2.0 one-shot parse / batch-corpus model.

## Consequences

**Easier**:
- Future PRs can cite "ADR-0012 § I-N" to anchor an algorithm
  choice rather than re-deriving the rationale.
- A reviewer skimming `aozora-veb/src/eytzinger.rs` knows
  immediately that vEB is a deliberate substitution candidate
  rather than a half-baked starter.
- The performance budget (table above) is auditable: each cell
  attaches to one or more I-N innovations; a regression in the
  budget surfaces which innovation needs revisiting.

**Harder / accepted cost**:
- New ADR maintenance: any innovation we replace (e.g., Eytzinger
  → vEB) needs to update this ADR's pinned choice. Mitigated by
  the fact that each section is short (~100 words).
- The "shipped in Move 1-4" vs "pending follow-ups" split tracks
  the migration's current shape; once the fused engine lands the
  split flattens and this ADR re-organises.

## Alternatives considered

- **Don't pin algorithms in an ADR; let the source be the
  source of truth**. Rejected: source describes *what*, not *why*.
  An ADR captures "we chose Eytzinger over vEB because vEB is
  harder to implement and the cache benefit is within ε for our
  sizes" in a way no code comment naturally surfaces.
- **One ADR per innovation (12 separate documents)**. Rejected
  for now: the innovations tightly interact (the lex pipeline,
  AST, registry, and SIMD scanner are all one performance budget).
  A single ADR keeps cross-references one click away. If any
  innovation grows large enough to deserve its own ADR (e.g.,
  Aho-Corasick lands with a non-trivial table-build strategy),
  it can split out.
- **Defer until the fused engine ships.** Rejected: pinning the
  pre-fused-engine choices now keeps the byte-identical proptest
  contract honest as later commits replace each piece.

## References

- Plan file: `/home/yasunobu/.claude/plans/jazzy-jingling-gizmo.md`
  — Innovations I-1 through I-12, full text
- ADR-0009 (Clean layered architecture)
- ADR-0010 (Zero-copy AST + observable equivalence)
- ADR-0011 (Multi-target deployment)
- Khuong & Morin, "Array Layouts for Comparison-Based Searching"
  (2017) — Eytzinger reference paper
- Lemire & Langdale, "Parsing gigabytes of JSON per second"
  (2019) — simdjson structural scan reference
- Aho & Corasick, "Efficient string matching" (1975) —
  multi-pattern DFA reference
