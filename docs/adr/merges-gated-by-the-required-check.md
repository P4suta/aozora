---
date: 2026-10-22
---

# Merges gated by the required check

## Status

proposed

## Context

The main ruleset requires only `required`, which depends on `ci-success`.
Separate CodeQL and dependency review workflows do not feed `ci-success`, so their failures cannot block a merge.

## Decision

Merges to `main` are gated by the single `required` check, covering CI, CodeQL, and dependency review.
`ci-success` includes the `codeql` matrix and the pull-request-only `dependency-review` job.
`release-ready` gates publication through `release-fan-in` and is no longer a merge gate.
This partly supersedes [ADR-0051](0051-ci-and-release-ready-split-event-driven-release-fan-in-and-the-actions-concurrency-budget.md)'s separate `release-ready`, `ci-success`, and CodeQL merge checks and the merge-gate requirement in the [ADR-0042 amendment](0042-release-ready-is-the-publish-authority.md).

## Consequences

A Release PR can merge before `release-ready` passes; a failing `release-ready` leaves the version bump unpublished until fixed.
Label-only pull request events rerun CodeQL.
Weekly CodeQL analysis is removed; pushes to `main` still run it through `ci`.
