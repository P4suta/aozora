# 0010. Zero-copy AST and observable equivalence

- Status: accepted
- Date: 2026-04-25
- Deciders: @P4suta
- Tags: architecture, ast, lifetimes, purity, 0.2.0

## Context

ADR-0008's "pure-functional 7-phase lexer" decision sets the lex
pipeline's correctness contract as: **every phase is a pure function
`fn(input) -> output` with no shared mutable state**. Each phase is
unit-testable in isolation, replay-deterministic, and structurally
composable. This was load-bearing for the 0.1 design.

Two pressures meet in 0.2.0:

1. **Throughput**: the four materialised IRs and per-node
   `Box<AozoraNode>` heap allocations are the dominant cost (the
   profiling work in the 0.2.0 plan attributes ~40% of parse time to
   them combined). Remediation requires (a) a fused single-pass
   pipeline over an arena and (b) AST nodes that borrow from source
   instead of owning copies.
2. **Type-state**: the seven phase functions exchange `Vec<Token>`,
   `Vec<PairEvent>`, etc. — types whose validity depends on having
   been produced by the immediately preceding phase. The current
   design relies on convention; nothing in the type system prevents
   passing a hand-built `Vec<PairEvent>` to a later phase with a
   half-formed pairing.

A zero-copy arena-allocated AST forces both an end to per-phase
function composition (the arena is shared mutable state by design)
and a lifetime parameter `'src` that propagates through the entire
AST and Tree types.

## Decision

Replace the contract "every phase is a pure function" with a strictly
weaker but operationally sufficient contract:

> **Observable equivalence:** `lex(src)` is a pure function from
> source text to `LexOutput` *as observed externally*. Repeated calls
> on byte-identical input MUST produce byte-identical output. The
> internal pipeline may use mutable arena allocation, fused
> iterators, type-state transitions, and SIMD-driven scratch buffers
> as long as this property holds.

Concrete adoption:

1. **Single lifetime parameter `'src` on the AST.** `AozoraNode<'src>`,
   `Content<'src>`, registry entries, and tree references all borrow
   from the source string owned by `Document<'src>`. `Box<str>` /
   `Box<AozoraNode>` are deleted from the AST schema.
2. **`Document<'src>` owns a `bumpalo::Bump`.** Every node produced by
   `Document::parse` is bump-allocated into that arena. When the
   `Document` drops, the arena drops as a unit; node `Drop` impls
   never run.
3. **Type-state encoding for the lex pipeline's internal stages.**
   Stage transitions are functions `Raw<'src> → Scanned<'src> →
   Classified<'src> → Built<'src>`, each consuming the previous
   state. Out-of-order use is a compile error.
4. **Owned escape hatch.** `aozora-syntax` provides `OwnedAozoraNode`
   plus `AozoraNode::to_owned()` for callers that need `'static`
   (serde serialisation, downstream tools that buffer parsed trees
   across documents). Conversion is opt-in; the default is borrowed.
5. **Property test: `parse(src) == parse(src)` byte-by-byte.** A
   proptest harness in `aozora-test-utils` pins observable
   equivalence on randomly generated aozora-shaped input, run on
   every CI build.

## Consequences

**Easier**:
- AST tree memory drops from ~100 KB to ~30-40 KB on 罪と罰 (3×
  reduction) once `Box<str>` / `Box<AozoraNode>` are gone.
- Per-document allocator pressure drops by ≥ 2,000 calls (one bump
  reservation vs ~2,700 individual `Box::new`s).
- Phase ordering bugs become compile errors instead of silent logical
  failures.
- Renderers that need to copy text into the output (HTML, serialize)
  can write `&'src str` directly without an intermediate `to_string()`.

**Harder / accepted cost**:
- Lifetime annotations propagate to every consumer of the AST. We
  accept the ergonomic cost because the alternative (boxed-everything
  AST) costs throughput we're explicitly trying to recover.
- Downstream serialisation (serde) requires `OwnedAozoraNode` for
  long-lived buffers. The `to_owned()` helper carries the conversion
  cost only when serialisation actually happens.
- The lex pipeline can no longer be unit-tested phase-by-phase in
  the way 0.1 supported. We compensate with (a) byte-identical
  observable-equivalence proptests, (b) corpus sweep regression
  gates, and (c) per-stage tests inside `aozora-lex` that exercise
  individual stages while still using the fused public entry point.
- ADR-0008's contract wording is replaced — that ADR will get a
  follow-up to either supersede the "every phase pure" line or
  re-frame it as historical.

## Alternatives considered

- **Keep `Box<AozoraNode>` and just add bumpalo internally**: would
  recover allocator pressure but not the per-node memory overhead, and
  would still leave `Box<str>` for inline content. Rejected: half a
  solution at the cost of the same lifetime story.
- **`AozoraNode<Cow<'src, str>>`**: more flexible (callers choose
  borrowed vs owned per-node), but adds an indirection on every text
  access for negligible benefit in practice — 99%+ of parsed nodes
  are produced from the source and borrow naturally.
- **`Arc<str>` instead of `&'src str`**: would let nodes outlive the
  source but adds a refcount per node. Rejected because callers that
  need detached storage already pay through `OwnedAozoraNode`, and the
  refcount tax falls on the common case.
- **Rowan-style green tree (lossless CST)**: would give stable spans
  and structural sharing for free, but inflates per-document memory
  ~50× and requires a parallel rewrite of the comrak post-process
  step (which knows about the current `AozoraNode` shape). Reserved
  for an eventual 0.3 incremental-LSP pivot if the use case demands
  it.

## References

- Plan file: `/home/yasunobu/.claude/plans/jazzy-jingling-gizmo.md`
  — Innovation I-3 (type-state) and I-4 (arena + zero-copy AST)
- ADR-0008 (Aozora-first lexer pipeline) — the contract this ADR
  weakens from "phase-pure" to "observable-equivalence"
- ADR-0009 (Clean layered architecture) — the surrounding refactor
- bumpalo: <https://docs.rs/bumpalo>
- Khuong & Morin, "Array Layouts for Comparison-Based Searching"
  (2017) — companion paper for `aozora-veb`'s Eytzinger choice
