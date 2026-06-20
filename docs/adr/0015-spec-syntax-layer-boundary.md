# 0015. The spec / syntax layer boundary

- Status: accepted
- Date: 2026-06-21
- Deciders: @P4suta
- Tags: architecture, crates

## Context

The parser splits its foundational vocabulary across two crates,
`aozora-spec` and `aozora-syntax`. The boundary between them has been
load-bearing but never written down, so recent refactors (the wire-tag
single-authority pass in #161, the render-class consolidation in #162)
each had to re-derive "does this enum / table belong in spec or syntax?".
The rule is worth recording before it drifts.

## Decision

Treat the two crates as distinct layers.

- **`aozora-spec` — values & tables.** Coordinate types (`Span`,
  `SourceOffset` / `NormalizedOffset`), the PUA `Sentinel` codepoints,
  `PairKind`, `Diagnostic`, and the slug catalogue (`SLUGS`,
  `roman_slug`, `SlugFamily`). It carries **no `aozora-*` runtime
  dependency** — it is the leaf that every other crate may depend on.

- **`aozora-syntax` — AST & semantic kinds.** The borrowed AST (`Node`,
  `Container`, the `NodeRef` registry) and the classifier's semantic
  enums (`NodeKind`, `ContainerKind`, `BoutenKind`, `HeadingKind`,
  `EmphasisKind`, …). It depends on `aozora-spec` (plus `aozora-veb` for
  the registry index and `aozora-encoding` for gaiji).

The dividing question is **who owns the mapping**:

- *enum → canonical 青空文庫 keyword* lives in **syntax** (e.g.
  `BoutenKind::keyword()`), because the enum is a syntax concept.
- *keyword → romaji slug* lives in **spec** (`roman_slug`), because the
  slug catalogue is a stable value table that editors and the renderer
  consume alike.

This documents the **current** arrangement; nothing is being moved. The
ADR exists so future additions land on the correct side by default.

### On `aozora-veb`

`aozora-syntax` depends on `aozora-veb` and holds it as a private field of
the borrowed registry (a van-Emde-Boas-style index backing `node_at`).
That is an ordinary dependency in the layer direction (syntax → veb), not
a boundary violation: `veb` is a generic data-structure crate with no
notion of Aozora notation, so it sits below syntax just as spec does, and
stays a standalone leaf rather than being folded into another crate.

## Consequences

- A new value table or coordinate type goes in `aozora-spec`; a new AST
  node or semantic kind goes in `aozora-syntax`.
- `aozora-spec` must stay free of `aozora-*` runtime dependencies. A spec
  item that needs a syntax type is the signal it was misplaced (or the
  dependency direction is wrong).
- The boundary is by convention, not enforced by a tool. This ADR is the
  reference; reviewers apply it when new enums or tables are added.
