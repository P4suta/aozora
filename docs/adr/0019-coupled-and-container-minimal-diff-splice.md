# 0019. Coupled and container minimal-diff splice

- Status: accepted
- Date: 2026-06-25
- Deciders: @P4suta
- Tags: architecture, api
- Supersedes: [ADR-0018](0018-minimal-diff-splice-and-source-region-ownership.md)

## Context

ADR-0018 landed the foundation of the minimal-diff edit splice (#202): a
total source-region tiling ([`Tree::owned_regions`]) and a single-region
splice for self-contained nodes. It deliberately deferred the regions whose
ownership is **split** (a non-adjacent [`ForwardOrigin::Referenced`] forward,
a heading hint, a margin note — the displayed literal lives in a separate
upstream run) or **paired** (a container open/close), reporting them
`Deferred` with a "lands in a later phase" reason.

This ADR finishes the splice core. The goal stated for #202 — and the explicit
ask — is to *stop deferring*: every classified region should be terminally
characterized and coherently editable (or honestly declined), with no
"phase N" placeholder left in the model. Two facts make that reachable
without enlarging the AST:

1. The coupling of a forward reference is exactly the irreducible
   [`ForwardOrigin`] provenance the coremodel epic already materialized
   (ADR-0003, #229). Nothing new must be stored to know a region is coupled.
2. A container's pairing is the structural nesting already present, in source
   order, in [`Tree::source_nodes`]. The open↔close link can be re-derived in
   source coordinates with no recourse to the normalized-coordinate
   `container_pairs` table.

## Decision

### Terminal classification

[`SpliceSafety`] is reshaped from `{Safe, Deferred(reason)}` to a terminal
trichotomy (the foundation API is unreleased, so this is free):

- **`Direct`** — the region fully owns its rendered content (a self-contained
  node, or plain interstitial text). Replacing its bytes is a complete edit.
  This subsumes the foundation's `Safe` *and* its `Deferred(Interstitial)`:
  plain text is the most directly editable region of all.
- **`Coupled(CoupledKind)`** — a coherent edit spans a *derived partner*: the
  upstream literal of a forward reference / heading hint / margin note, or the
  matching marker of a container.
- **`Opaque`** — a future [`Node`] variant this build does not classify
  (forward-compat only; never produced by a construct this version
  understands). Declined rather than guessed.

There is no "deferred to a later phase" state. `Coupled` regions are editable
now; the corpus-attested hard cases (below) surface as a typed *edit* error,
not a region class.

### Derive every coupling on demand

No link is stored on any AST node. [`Tree::coupling`] recovers a region's
partner span(s) from the tables that already exist:

- **Container** — a depth-stack walk over `source_nodes` (the same LIFO the
  normalizer itself uses) pairs open↔close *directly in sanitized-source
  coordinates*. ADR-0018's "normalized↔source coordinate bridge" collapses to
  "re-derive the pairing in source space"; the normalized `container_pairs`
  table is never consulted.
- **Forward / heading hint / margin note** — the directive's quoted target is
  read off the node payload and relocated as its *unique* upstream plain
  occurrence (the coupled literal). A target-text change rewrites both the
  bracket and that occurrence; the irreducible cases — an ambiguous referent
  (more than one occurrence), a ruby-base literal (the occurrence is inside a
  classified construct, not a lone plain run), or a multi-segment target — are
  declined rather than guessed.

### Propose, verify in a scoped context

[`Tree::splice`] replaces the foundation's `splice_source`. For a `Direct`
region it is a byte replacement. For a `Coupled` region it **proposes** a
candidate (rewriting the derived partner — the matching container close for a
new open; the upstream literal for a forward target change) and **verifies** it
by re-parse before returning it. The parser is the single source of truth for
"what couples to what": this layer proposes, the parser confirms. A candidate
that does not re-parse to the intended construct is declined with
[`SpliceError::Unverifiable`] rather than emitted as a byte-valid but
semantically desynced edit. This is deliberately *not* a second copy of the
classifier's matching rules — re-implementing them would reintroduce the
non-local look-back the scope-free core removed, and would drift from the
parser.

**The verification is scoped, never whole-document.** A minimal-diff edit must
not cost `O(document)`; verifying it by re-parsing the whole document would
defeat the point and make an interactive editor's per-edit cost scale with file
size. Instead each edit parses only the construct it touches:

- A container marker is a self-delimiting `［＃…］` bracket, so its standalone
  parse equals its in-context parse. The new open's `RegionFormat` is recovered
  by parsing the *replacement marker alone*; the matching close is then its
  canonical partner ([`RegionClose::of`]); and because the markers replace 1:1
  the document's nesting is unchanged — so the edit is correct by construction
  with no whole-document re-parse (`O(marker)`).
- A forward / heading-hint / margin-note bracket is *not* self-delimiting (it
  references upstream text), so it is verified against the node's own target in
  a minimal context (`<target><replacement>`) — the smallest window that
  re-forms the reference — never the whole document.

The existing open's format and the container pairing are read off the
*already-parsed* tree (`source_nodes`, an `O(log n)` lookup), adding no parse.

`Document::edit_region` is the ergonomic `Document`-returning wrapper, layered
over the existing `Document::edit` (which is unchanged). It operates in
sanitized coordinates, so its result is byte-identical to `edit` on inputs
that triggered no sanitize rewrite, and equal to `splice + Document::new`
otherwise (a sanitized-coordinate region cannot be applied to un-sanitized
bytes).

### Honestly irreducible cases

Some coupled *edits* cannot be made coherent and are declined (the region is
still `Coupled`; the specific edit returns `Unverifiable`):

- an **ambiguous** forward referent (the target occurs more than once upstream
  with no unique occurrence);
- a **、-joined multi-target** (`［＃「A」「B」に傍点］`) whose rendered target is
  not a source substring;
- a **ruby-base** target literal (`我《われ》…我［＃「我」に傍点］`), which cannot be
  carved from a plain interstitial run.

These are terminal truths surfaced by the verify step, not deferrals.

## Consequences

- Editor surfaces get a coherent minimal-diff edit for every construct
  through the `aozora` front door — `Direct` byte edits, `Coupled` two-region
  edits — with the parser guaranteeing no silent desync.
- The splice model is the dual of the parser's classification, derived
  entirely on demand. The AST gains no field; afm / aozora-proof, which pin
  `borrowed::ForwardFormat`, need no follow-up.
- Rendering and serialization are untouched: `to_html`, `to_source`,
  `to_source_verbatim`, `source_nodes`, `container_pairs`, the conformance
  vectors, and the round-trip fixed point are byte-identical. Only the
  (unreleased) splice API surface changed.
- The corpus tiling gate (`tests/corpus_splice_tiling.rs`) is strengthened:
  the identity splice of *every* region — `Direct` by byte replacement,
  `Coupled` through partner-derivation + scoped verification — must reproduce
  the verbatim source over all 17,889 corpus documents, and no region is
  `Opaque`. Scoped verification keeps this affordable for every region of every
  document (an early whole-document-re-parse implementation was `O(document)`
  per edit and far too slow to run for the full corpus).

## Alternatives considered

- **Store the partner link on the node** (an upstream-literal span on
  `ForwardFormat`, a close span on the container open). Rejected: it re-bloats
  the Copy AST the coremodel epic purified, and breaks the afm pin, to cache
  something cheaply derivable.
- **Re-implement the classifier's matching rules inside the splice** to decide
  coupling without re-parsing. Rejected: a second source of truth that drifts
  from the parser and revives the non-local look-back the scope-free core
  removed.
- **Keep `Deferred` and add a capability query** (the conservative, fully
  additive route). Rejected in favour of the terminal `Direct/Coupled/Opaque`
  model: the brief was to finish the core, and a self-describing classification
  with no "later phase" state is the more honest shape. The unreleased API
  makes the reshape free.
- **Pair containers via the normalized `container_pairs` table.** Rejected:
  it forces a normalized↔source coordinate cast (a deliberately guarded
  operation); the source-order depth-stack recovers the pairing directly.

## References

- `crates/aozora/src/splice.rs`,
  `crates/aozora/tests/corpus_splice_tiling.rs`,
  `crates/aozora/tests/splice_api.rs`.
- `crates/aozora-render/src/serialize.rs` (`container_close_source` — the
  single source of truth for close-marker spelling the splice reuses).
- ADR-0018 (foundation: tiling + single-region splice), ADR-0003 (spec:
  forward provenance), ADR-0015 (spec/syntax layer boundary).
- Issues #202 (this work), #189 (coremodel-purification umbrella), #229
  (`ForwardOrigin` provenance enum).

[`ForwardOrigin`]: https://docs.rs/aozora/latest/aozora/enum.ForwardOrigin.html
[`Node`]: https://docs.rs/aozora/latest/aozora/owned/enum.NodeOwned.html
[`Tree::owned_regions`]: https://docs.rs/aozora/latest/aozora/struct.Tree.html#method.owned_regions
[`Tree::coupling`]: https://docs.rs/aozora/latest/aozora/struct.Tree.html#method.coupling
[`Tree::splice`]: https://docs.rs/aozora/latest/aozora/struct.Tree.html#method.splice
[`SpliceSafety`]: https://docs.rs/aozora/latest/aozora/enum.SpliceSafety.html
[`SpliceError::Unverifiable`]: https://docs.rs/aozora/latest/aozora/enum.SpliceError.html
