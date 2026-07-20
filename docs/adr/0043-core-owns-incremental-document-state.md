# ADR-0043: Core owns incremental document state

- Status: Accepted
- Date: 2026-07-19
- Supersedes: ADR-0041

## Context

Moving parsing behind `Document` removed public pipeline stages, but the
language server still maintained a second editable rope, paragraph partition,
tree-sitter forest, gaiji index, and semantic parse cache. Each edit therefore
had two parsing authorities and two opportunities to disagree about text,
coordinates, or publication order.

The core now retains immutable paragraph snapshots and can update one changed
paragraph while sharing unaffected parsed state. Keeping an editor-only
incremental engine no longer provides a capability the public document model
lacks.

## Decision

`Document` is the only mutable Aozora parse state in every in-process channel.
It owns the rope after the first edit, validates batches in pre-edit source
UTF-8 byte coordinates, updates the affected parsed paragraph, and publishes an
immutable `Snapshot`.

The language server stores one `Document` and one atomically published view of
its current `Snapshot`. Diagnostics, semantic tokens, rename, rendering,
gaiji lookup, and source projections read that same snapshot. LSP UTF-16 line
indexes are protocol adapters derived lazily from snapshot source; they do not
parse Aozora notation.

There is no language-server tree-sitter state, paragraph parser, semantic
parse cache, or independent edit splice. The separately distributed
tree-sitter grammar remains a full-spec consumer for hosts that explicitly
choose tree-sitter.

## Consequences

An accepted edit is immediately visible with matching text and semantic
projections. Debouncing controls diagnostic publication only; it does not
defer or repeat parsing.

Incremental/full equivalence and edit performance are core contracts. Editor
tests exercise the same public `Document` path used by other bindings.
