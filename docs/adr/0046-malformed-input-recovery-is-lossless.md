# ADR-0046: Malformed input recovery is lossless

- Status: Accepted
- Date: 2026-07-19

## Context

The parser accepts malformed notation and reports diagnostics so editors can
continue operating on incomplete text. Reconstructing that text from a
partially recovered semantic tree can invent a missing delimiter, discard an
explicit ruby base marker, or normalize a mismatched container close. Those
changes violate the source-preservation contracts in ADR-0003, ADR-0019, and
ADR-0030.

The conformance vectors had encoded some of those reconstructed forms as
expected output. They also carried an accent diagnostic offset in decomposed
text coordinates even though every public span now names the original UTF-8
source.

## Decision

The default source serializer preserves the original source byte-for-byte when
parsing reports an unclosed bracket, a mismatched container close, or nested
ruby recovery. This includes preparse transformations such as BOM removal,
newline normalization, and accent decomposition: recovery never changes the
public source or its coordinate space.

Explicit directive normalization remains an author-requested rewrite and may
produce canonical source.

Gaiji projection spans use the public original-source `Span` type. The
projection exposes immutable accessors rather than platform-sized internal
offsets or mutable payload fields.

Conformance expectations for malformed container and nested-ruby inputs record
the original source. Accent diagnostic spans record original-source byte
offsets. The Rust parser and tree-sitter grammar must both accept the same
vectors without parser errors; their tree projection shapes need not be
identical.

## Consequences

`Snapshot::to_source` and its default-options equivalent are safe persistence
paths for incomplete editor buffers. `Snapshot::to_source_verbatim` remains the
explicit no-serialization view, but malformed recovery no longer requires
callers to switch APIs to avoid data loss.

The public edit, diagnostic, node, gaiji, and wire projections share one
original UTF-8 byte coordinate contract. Generated host types use the same
bounded span representation.

Spec-vector changes are made in the external specification repository and then
vendored by its pinned commit; the local copy cannot carry an unpinned
exception.

## Alternatives considered

- Preserve the recovered semantic serialization and document its differences.
  Rejected because an editor save would mutate text the user did not edit.
- Return post-sanitize text during recovery. Rejected because it loses BOM,
  newline, and accent spelling while claiming source-coordinate fidelity.
- Add recovery flags to serialization options. Rejected because losslessness is
  the safe default and callers should opt into rewriting, not into preservation.
