# 0009. Clean layered architecture for aozora 0.2.0

- Status: accepted
- Date: 2026-04-25
- Deciders: @P4suta
- Tags: architecture, refactoring, 0.2.0

## Context

`aozora` 0.1 evolved organically: a 7-phase lexer (sanitize / tokenize /
pair / classify / normalize / registry / validate) shared between
`aozora-syntax` (Span), `aozora-lexer` (sentinels, TriggerKind, PairKind,
Diagnostic, the seven phase implementations), and `aozora-parser` (which
wraps everything plus serialize, html, segment-parallel, and
incremental). The result works and is fast (~96 MB/s per core on a
16-core x86-64 host), but it has four asymmetries that compound as the
project grows:

1. **Four materialised intermediate IRs** — `Vec<Token>`,
   `Vec<PairEvent>`, `Vec<ClassifiedSpan>`, `String + Registry` —
   cause the L1/L2 cache to be touched four times per parse.
2. **`Box<AozoraNode>` per AST node** — every recognised construct
   forces a global-allocator hit. 罪と罰 (~2,700 inline nodes)
   measures ~100 KB of allocator-fragmented heap per parse.
3. **Single `parse(&str)` front door collects scan/lex/build/render/
   parallel/incremental into one crate**, blurring layer boundaries
   and forcing every consumer to depend on the entire pipeline.
4. **Native-Rust-only delivery** — no path to a browser (WASM), to
   data-science workflows (Python), or to other hosts (C ABI).

The 0.2.0 release breaks API compatibility deliberately to address
all four asymmetries in a single coordinated reshape. ADR-0001 (zero
parser hooks into comrak) and ADR-0008 (Aozora-first lexer pipeline)
remain non-negotiable; the changes here sit *above* and *around* those
two pillars without disturbing them.

## Decision

Replace the seven-phase organisational unit with a **four-layer
architecture**, each layer in its own crate (or set of crates) with a
single declared responsibility:

| Layer    | Responsibility                                | Crates                                          |
|----------|-----------------------------------------------|-------------------------------------------------|
| Source   | Specification + low-level tools               | `aozora-spec`, `aozora-encoding`, `aozora-scan` |
| Lex      | Fused streaming lexer (former 7-phase)        | `aozora-lex`                                    |
| Tree     | Semantic AST data definitions, no logic       | `aozora-syntax`                                 |
| Surface  | User-facing APIs and target-specific drivers  | `aozora` (meta), `aozora-render`, `aozora-parallel`, `aozora-cli`, `aozora-wasm`, `aozora-ffi`, `aozora-py` |

The public API exposes exactly two types: `Document<'src>` (owner of
source string and a per-document arena) and `AozoraTree<'src>` (borrow
view into the parsed result). The seven phase functions and four
intermediate IR types become internal implementation details of
`aozora-lex` — invisible from outside the crate.

The migration runs in four mergeable Moves (1 / 2 / 3 / 4):
spec extraction → lex layer fusion → surface split → multi-target
drivers. Each Move is independently reviewable and lands its own
verification gate (byte-identical proptest equivalence with the prior
implementation).

## Consequences

**Easier**:
- New consumers only learn `Document` + `AozoraTree`; phase names
  disappear from the public surface.
- Per-crate dependency declarations match per-crate responsibility,
  so a consumer that wants only diagnostic types pulls `aozora-spec`
  alone (no lexer, no parser).
- Adding a new renderer (TeX, EPUB) is one crate's worth of work
  against the visitor surface in `aozora-render`, with zero touch to
  the lex/spec/syntax layers.
- Multi-target shipping: each driver crate (`aozora-wasm`,
  `aozora-ffi`, `aozora-py`) is a thin facade over the same `aozora`
  meta crate.
- Cache-line hits drop because the four-IR pipeline collapses to a
  single fused stream (Move 2).
- Allocator hits drop because every parse owns one bumpalo arena that
  drops as a unit (Moves 1.4 + 2).

**Harder / accepted cost**:
- Crate count grows from 7 to ~15. We accept the navigation cost
  because each crate's responsibility is now obvious from its name
  ("aozora-scan scans, aozora-render renders").
- Downstream consumers (`afm`, etc.) must migrate from
  `aozora_parser::parse` to `aozora::Document::parse`. We provide
  `to_owned()` helpers on `AozoraNode<'src>` for callers that need
  `'static` data.
- Compile times increase slightly (more crates = more compilation
  units). Offset by per-crate parallelism in `cargo build`.

## Alternatives considered

- **Incremental pessimisation (keep 0.1 shape, add features in
  place)**: Rejected because each of the four asymmetries reinforces
  the others — fixing only one (e.g., adding bumpalo without splitting
  the parser crate) leaves the other three to keep accumulating
  technical debt.
- **Rust-analyzer-style rowan + salsa**: Rejected for 0.2.0 — see
  ADR-0010's "Alternatives". Reserved for an eventual 0.3 incremental
  pivot if real-world LSP usage warrants it.
- **Single mega-crate `aozora`**: Rejected because it forces every
  consumer to compile and link the full pipeline even when they only
  need a diagnostic struct or a sentinel constant.

## References

- Plan file: `/home/yasunobu/.claude/plans/jazzy-jingling-gizmo.md`
- ADR-0001 (zero parser hooks)
- ADR-0008 (Aozora-first lexer pipeline) — the lexer purity contract
  this architecture preserves
- ADR-0010 (zero-copy AST + observable equivalence) — the lifetime
  + purity contract that arrives with Move 1.4
- ADR-0011 (multi-target deployment) — Move 4 driver-crate scheme
- ADR-0012 (algorithmic baseline) — concrete algorithm choices
  (simdjson-style scan, Eytzinger registry, Aho-Corasick annotations)
