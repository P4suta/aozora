# ADR-0050: Immutable releases are assembled as drafts

- Status: Accepted
- Date: 2026-07-21

## Context

GitHub makes a published immutable release's tag and assets unmodifiable. The
native publisher previously published the release immediately, while the
Extism publisher attached its assets later from a separate workflow. That
ordering cannot complete once immutability is enforced.

## Decision

The native publisher creates a draft release and attaches the native and Go
artifacts. The Extism publisher accepts only that draft and attaches its exact
qualified artifacts. Registry publishers may be retried against the same tag
and commit while the release remains a draft.

A maintainer publishes the draft only after every intended registry and asset
has been verified. No workflow overwrites release assets after publication.

## Consequences

Publishing the draft is the final irreversible release action. A failed channel
leaves a recoverable draft instead of an incomplete immutable release, and the
published release attestation covers the complete artifact set.
