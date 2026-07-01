# 0018. Minimal-diff splice and source-region ownership

- Status: superseded by [ADR-0019](0019-coupled-and-container-minimal-diff-splice.md)
- Date: 2026-06-24
- Deciders: @P4suta
- Tags: architecture, api

## Context

The coremodel-purification epic (#189) finished the scope-free core: the
lowering pass tiles the sanitized source into classified `source_nodes`,
and the one surviving per-node fact a forward reference cannot shed —
whether its target literal is owned by the node or lives upstream — is
materialized as the irreducible [`ForwardOrigin`] provenance (ADR-0003
on the spec side, parser #229).

The last pillar that epic explicitly *deferred* (#202) is **minimal-diff
editing**. An editor surface that "adds ruby to this word" or "changes
this heading level" wants the resulting source to differ from the
original by the smallest possible diff — it must **not** reflow the whole
document to canonical form (`to_source`), which would rewrite the
author's verbatim formatting everywhere. Doing that needs two facts the
provenance model can now supply but had never been stated as a contract:

1. **Which source bytes does each node own?** Today `source_nodes` tile
   the source "contiguously by construction" — relied on by `aozora-cst`,
   pandoc, and the wasm surface — but the tiling-completeness was never an
   asserted invariant.
2. **Can a node be edited by replacing its bytes alone?** Not always: a
   non-adjacent forward reference's displayed literal lives in a separate
   upstream run (`ForwardOrigin::Referenced`), so editing only its bracket
   desyncs the reference.

## Decision

Add a **source-region ownership** layer in the `aozora` core crate
(`crates/aozora/src/splice.rs`), as a purely additive read surface plus a
minimal-diff splice primitive.

**Ownership tiling.** [`Tree::owned_regions`] projects `source_nodes`
into a *total, gap-free, ordered, non-overlapping* tiling of the sanitized
source: one `OwnedRegion` per classified node plus the interstitial plain
runs between them. Concatenating every region's bytes reproduces
[`Tree::to_source_verbatim`] exactly. This lives in core because the
ownership truth is `SourceNode.source_span` (sanitized coordinates); the
rowan-backed `aozora-cst` is a *re-projection* of the same table behind
the `cst` feature, not the base model.

**Splice safety, derived from provenance.** Each region carries a
`SpliceSafety`, a pure function of its `NodeRef` and (for a forward leaf)
its `ForwardOrigin`:

- **Safe** — the region fully owns its rendered content, so replacing its
  bytes is a complete edit: ruby, gaiji, leaf directives, a
  `ForwardOrigin::Reclaimed` forward (the literal was pulled into the
  node), and a promoted heading (its referent line was reclaimed).
- **Deferred** — ownership is *split* (`ForwardOrigin::Referenced`
  forward, a non-promoted heading hint, a margin note — the displayed
  literal lives in a separate upstream region) or *paired* (a container
  open/close marker, paired in normalized coordinates). A coherent edit
  needs multi-region coordination.

[`Tree::splice_source`] returns the minimal-diff source for a `Safe`
region (`verbatim[..start] + replacement + verbatim[end..]`) and declines
a `Deferred` region with a typed error rather than emitting a byte-valid
but semantically incomplete edit.

**Scope of this slice.** Minimal-diff *serialization* only. Incremental
*re-parse* (reusing the unaffected tree across an edit) is a separate,
larger effort and is **not** attempted here — the parser is single-digit
milliseconds on real corpus documents, so there is no current performance
pressure, and incremental reuse would require reworking the `!Sync` arena.
The whole layer is additive: every existing output (`to_html`,
`to_source`, `to_source_verbatim`, `nodes`, conformance vectors, the
round-trip fixed point) is byte-for-byte unchanged, and `Document::edit`
is untouched.

**Gate.** `tests/corpus_splice_tiling.rs` asserts, over every
`$AOZORA_CORPUS_ROOT` document, that the tiling is a complete cover and
that the identity splice of every `Safe` region reproduces the verbatim
source — extending `aozora-cst`'s property-test lossless invariant to the
full corpus (17,889 works at time of writing).

## Consequences

- Editor surfaces get a node→source-region map and a working minimal-diff
  splice for the common (self-contained) case, through the `aozora` front
  door, with no rowan dependency.
- The irreducible `ForwardOrigin` provenance is exactly what makes the
  Safe/Deferred decision local and total — the split-ownership cases are
  precisely the ones the provenance bit flags.
- The phased roadmap under #202 continues in **ADR-0019** (coupled splice for
  `Referenced` forwards / heading hints / margin notes, and container splice —
  the terminal `Direct`/`Coupled`/`Opaque` model that supersedes this ADR's
  `Safe`/`Deferred`). Incremental re-parse (reusing the unaffected tree across
  an edit) remains a separate performance concern, gated behind the v0.5.0
  release (#99), the `!Sync` arena rework, and a real consumer (the LSP
  `ParseCache`'s cache-hit path) — not part of the splice *model*.
- Cost: split / paired regions are not yet editable through this API; a
  consumer that needs them edits the verbatim bytes directly until the
  coupled-splice phase lands.

## Alternatives considered

- **Put the splice at the rowan CST layer (`aozora-cst`).** Rejected as
  the home: the CST is a projection of `source_nodes`, the ownership truth
  is in core, and rowan is deliberately behind the `cst` feature so plain
  library consumers don't pull it in. The splice belongs in core; the CST
  re-projects it.
- **Re-serialize canonically for edits (`to_source`).** Rejected: it
  reflows the entire document to canonical form, discarding the author's
  verbatim formatting in untouched regions — the opposite of minimal-diff.
- **Treat `source_span` as the editable unit uniformly, ignoring
  provenance.** Rejected: a `Referenced` forward's literal lives upstream,
  so replacing only its bracket silently breaks the reference. Provenance
  is what separates self-contained from split ownership.
- **Build the full incremental re-parse engine now.** Rejected for this
  slice: speculative (no measured performance need), a large correctness
  surface, and blocked on the `!Sync` arena rework. Deferred to a later
  phase, driven by a concrete consumer.

## References

- `crates/aozora/src/splice.rs`,
  `crates/aozora/tests/corpus_splice_tiling.rs`,
  `crates/aozora/tests/splice_api.rs`.
- ADR-0003 (spec): canonical serialization forms and forward provenance.
- Issues #202 (this work), #189 (coremodel-purification umbrella), #229
  (`ForwardOrigin` provenance enum).

[`ForwardOrigin`]: https://docs.rs/aozora/latest/aozora/enum.ForwardOrigin.html
[`Tree::owned_regions`]: https://docs.rs/aozora/latest/aozora/struct.Tree.html#method.owned_regions
[`Tree::to_source_verbatim`]: https://docs.rs/aozora/latest/aozora/struct.Tree.html#method.to_source_verbatim
[`Tree::splice_source`]: https://docs.rs/aozora/latest/aozora/struct.Tree.html#method.splice_source
