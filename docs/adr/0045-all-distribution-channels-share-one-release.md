# ADR-0045: All distribution channels share one release

- Status: Accepted
- Date: 2026-07-19
- Supersedes: The independent editor cadence in ADR-0016

## Context

The VS Code extension used a hand-maintained version and a separate tag even
though it embeds the product CLI and consumes the same wire schema. That made
it possible to publish an editor artifact that did not identify the same
source, version, or release qualification as the other channels.

## Decision

The workspace version and `vX.Y.Z` tag identify every distribution channel.
Release packaging injects that version into the VS Code manifest. The checked-in
manifest carries only a non-release placeholder, so release-plz remains the
single version authority.

VS Code Marketplace, Open VSX, crates.io, npm, PyPI, GitHub native archives,
Extism, Go, and the hosted playground publish artifacts qualified by
`release-ready` for the tagged commit.

## Consequences

An editor-only release no longer exists. Any channel change participates in the
same Release PR, qualification result, tag, and retryable artifact set as the
rest of the product.
