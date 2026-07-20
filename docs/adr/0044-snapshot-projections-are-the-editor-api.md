# ADR-0044: Snapshot projections are the editor API

- Status: Accepted
- Date: 2026-07-19
- Supersedes: The public CST consequences of ADR-0003 and ADR-0018

## Context

The rowan CST and its query language duplicated source ownership, node
classification, coordinate mapping, and traversal already provided by
`Snapshot`. No shipped editor used either surface. Keeping them as optional
features made two independent editor models part of the supported SemVer
contract.

## Decision

`Snapshot` is the only editor-facing syntax projection. It exposes immutable
source-coordinate node views, diagnostics, pairs, literal markup, directives,
ruby, containers, gaiji resolution, rendering, and serialization.

The `cst` and `query` features and their rowan dependency are removed.
Consumers that need a custom index build it from stable snapshot views.
Tree-sitter remains a separately distributed lossless editing grammar, but it
does not define `aozora` semantics or own document state.

## Consequences

There is one source owner, coordinate contract, node classifier, and
incremental cache across the library and shipped editor clients. New editor
projections extend `Snapshot` instead of introducing another syntax tree.
