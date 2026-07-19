# ADR-0041: Public façade and editable documents

- Status: Accepted
- Date: 2026-07-19
- Supersedes: ADR-0016, ADR-0036

## Context

The workspace was reduced from eighteen publishable crates to three, but
workspace consumers still reached parse stages through feature-gated,
hidden exports. Cargo features and rustdoc visibility do not remove those
items from the compatibility surface. Stage-shaped APIs also forced editor
clients to own coordinate conversion and incremental-parser invariants.

## Decision

The only publishable packages are `aozora`, `aozora-cli`, and
`tree-sitter-aozora`. `aozora-cli` is binary-only.

`aozora` exposes parsing through `parse`, `Parser`, `Document`, and
`Snapshot`. A document owns source and parsed state. Edits use old-source
UTF-8 byte ranges; batches are sorted, disjoint, and atomic. Snapshots are
immutable, cheaply cloneable, and safe to move or share between threads.
Diagnostics and public spans use original-source byte coordinates.

Directive discovery and completion metadata are exposed through
`Catalogue`. Encoding exposes decoding and gaiji resolution, not pipeline
stages.

Sanitizing, tokenizing, pairing, classifying, allocation, sentinels,
normalized-coordinate maps, and incremental splice tables remain
implementation details. There are no development-only features that expose
them. Every public feature and dependency is a supported SemVer contract.
Benchmarks, fuzz targets, command-line tools, and workspace tasks drive the
same public entry points as external users.

Workspace gates verify the publishable package set, binary-only CLI shape,
package dependency closure, features, and packaged files.

## Consequences

The pre-release API is intentionally replaced without compatibility aliases.
Editor clients retain protocol and tree-sitter paragraph state but delegate
Aozora parsing, snapshots, diagnostics, rendering, serialization, and edits
to `Document`.

Private symbols may change without updating benchmark source. Stage costs are
attributed from profiler symbols instead of direct stage invocation.
