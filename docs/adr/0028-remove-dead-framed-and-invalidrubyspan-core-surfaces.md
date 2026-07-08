# 0028. Remove dead Framed and InvalidRubySpan core surfaces

- Status: accepted
- Date: 2026-07-08
- Deciders: @P4suta
- Tags: architecture, notation, parser, wire

## Context

Two `aozora-syntax` core enum surfaces were **declared and fully wired** —
serialization, kind projection, downstream `match` arms, corpus counters — yet
**never constructed from source** by any parser path. They were the reason the
45-family golden universe was only reachable to 41/45; [ADR-0027](./0027-core-parser-notation-purification.md)
and #454 documented them on the *measurement* side as `STRUCTURALLY_UNREACHABLE`
(a negative catalogue that makes "covered OR correctly-irreducible" provable).
This ADR is the *core* purification: remove them.

- **`DirectiveKind::InvalidRubySpan`** (doc: "a ruby span that couldn't be
  parsed cleanly"). Unwired since the bootstrap commit (`f16214c`, 2026-04-23).
  A whole-workspace search finds **no constructor** — only match arms (the
  Pandoc slug `"invalid-ruby-span"`, the corpus counter bucket) and one slug
  test. Its intended role was **superseded by the diagnostics channel**: on
  malformed ruby the classifier emits a structured `Diagnostic`
  (`empty_ruby_reading` / `nested_ruby`) and replays the raw bytes as plain text
  losslessly — it never mints a typed directive. It is removal residue.
- **`LineFormat::Framed(EnclosureKind)` → `NodeKind::Framed`**. Constructed only
  in two tests. It is a byproduct of the **symmetric per-scope enum** design
  introduced in [#207](https://github.com/P4suta/aozora/pull/207) (I9): every
  attribute got a variant in each scope sum (`ForwardAttr` / `LineFormat` /
  `RegionFormat`) whether or not a notation reached it. The `罫囲み` line spelling
  is in fact claimed by the paired container (`RegionFormat::Framed`,
  `containerOpen`) and the forward `「X」は罫囲み` by `ForwardAttr::Framed`
  (`Emphasis`), so the line scope has **no path**. Worse, it had an asymmetric
  round-trip: the serializer emitted `LineFormat::Framed → ［＃罫囲み］`, which the
  parser reads back as a *container-open* — a different node at a different scope.

This mirrors ADR-0027 exactly, which removed the dead
`RegionFormat::CombineUpright` / `RegionClose::CombineUpright` variants because
縦中横 "has no paired-range form" — the identical "no path at this scope" reason,
one scope over.

## Decision

**Clean-break remove both dead surfaces** and every downstream arm they fed
(`aozora-syntax` decl / `NodeKind::ALL` / `as_json_tag` / `kind()` /
`xml_node_name()`; the `aozora-render` source-spelling + HTML hooks; the
`aozora-pandoc` class + slug arms and their tests; the CLI `aozora kinds`
description; the `aozora-xtask` corpus counter + `ANN_KIND_LABELS` bucket; the
handbook `keigakomi.md` node page). The malformed-ruby behavior stays as-is
(diagnostic + lossless replay); the multi-line 罫囲み container stays documented
in `container.md`.

**Explicitly retained** — these share the `Framed` / enclosure vocabulary but are
**live from source**, so they are untouched:
`Format::Framed(EnclosureKind)` (constructed via `BlockStyles::iter_formats` for
the `、罫囲み` indent compound, and via the `ForwardAttr` / `RegionFormat`
projections), the whole `EnclosureKind` enum (all five kinds reachable through
`ForwardAttr::Framed` and `RegionFormat::Framed`), `ForwardAttr::Framed`,
`RegionFormat::Framed` / `RegionClose::Framed`, and the `ContainerKind` `"framed"`
wire tag.

## Consequences

- **Wire — `SCHEMA_VERSION` held at 2** (no bump). Neither tag was ever emitted,
  so no JSON bytes change and the JSON-Schema artefacts (`schema-*.json`, whose
  `kind` field is an open `{"type":"string"}`, not an enum) are byte-identical.
  The only generated diff is the TypeScript **`NodeKind`** union dropping the
  dead `"framed"` member; the live `ContainerKind` `"framed"` token stays. This
  is a source-only narrowing of a convenience type, so it follows the **#176**
  precedent (the `wire`→`json` rename: type surface changed, emitted bytes
  identical → schema version held, recorded as source-only BREAKING in the
  CHANGELOG). It **diverges from ADR-0027**, which recorded the never-emitted
  `combineUprightRange` tag removal under a `SCHEMA_VERSION` 1 → 2 bump — but
  that bump was *mandatory* for ADR-0027's concurrent `lineGothic` rename and
  `gothic` addition (both genuine emitted-byte changes); the tag removal merely
  rode along. #455 has no such concurrent change, so the wire version is honest
  to hold. Downstream `features = ["json"]` consumers (aozora-proof, afm) pin an
  immutable `aozora` version ([ADR-0017](./0017-ecosystem-dependency-pin-policy.md))
  and only need to update source that *names* the `"framed"` `NodeKind` member —
  no runtime behavior changes.
- **Family universe: 45 → 43.** `FAM_TOTAL` is computed from the array lengths
  (`NodeKind::ALL` ∪ non-Unknown `ANN_KIND_LABELS` ∪ `GAIJI_FORM_LABELS`), so it
  auto-follows; `STRUCTURALLY_UNREACHABLE` drops to `["warichu", "container"]`
  (the two genuinely-live post-fold nodes) and the golden gate stays green with
  `covered ∪ unreachable = 43`.
- **Spec vectors + corpus baselines: unchanged.** The parser output is
  byte-identical, so no vector re-vendor is needed (the `keigakomi_inline_framed`
  vector already expects a `directive` decline, not a `framed` node), and the
  corpus gates (audit / digest / catalogue) stay at their current values (both
  tags were always count 0).

## Alternatives considered

- **Wire them up instead of removing.** Give `InvalidRubySpan` a real producer
  for malformed ruby, and `LineFormat::Framed` a genuine single-line `罫囲み`
  source form. Rejected: both roles are already owned — malformed ruby by the
  diagnostics channel (a structured, span-carrying report is strictly better
  than a lossy typed directive), and every `罫囲み` spelling by the live
  container / forward paths. Wiring them would invent notation the corpus does
  not attest and re-introduce the asymmetric round-trip.
- **Bump `SCHEMA_VERSION` 2 → 3 for precedent-consistency with ADR-0027.**
  Rejected: see the Wire consequence — ADR-0027's bump was driven by real
  emitted-byte changes, not the tag removal; #176 is the controlling precedent
  for a surface-only change with identical emitted bytes. Bumping would signal a
  wire break that does not exist.
- **Keep the dead variants behind the `STRUCTURALLY_UNREACHABLE` catalogue.**
  Rejected: the catalogue is a *measurement*-side accommodation for live-but-
  pre-fold nodes (`warichu` / `container`); using it to permanently park dead
  core surface normalises exactly the unreachable-variant clutter the core-purity
  discipline exists to prevent.

## References

- Issue #455 (this work); [ADR-0027](./0027-core-parser-notation-purification.md)
  (the `CombineUpright` removal precedent and the measurement-side
  `STRUCTURALLY_UNREACHABLE` catalogue), #454 (populated that catalogue),
  [ADR-0022](./0022-notation-hygiene-layer-roles.md) (layer roles), #207 (the
  symmetric per-scope `LineFormat` design), #176 (wire-version-held-on-surface-
  only-change precedent).
- Plan: `.claude/plans/issue-455-optimized-shell.md`.
- Evidence: `aozora-syntax/src/{lib,format,node_kind,ast/payload}.rs`,
  `aozora-render/src/spelling/{source,html}.rs`, `aozora-pandoc/src/project.rs`,
  `aozora-xtask/src/corpus.rs` (the family-coverage gate).
