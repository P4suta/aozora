# 0040. Run mutation sweeps only in Docker

- Status: accepted
- Date: 2026-07-19
- Deciders: @P4suta
- Tags: infra, ci, testing

## Context

ADR-0031 added a host-native parallel lane to accelerate mutation campaigns.
The lane and the Docker sweep both forced all workers through one
`CARGO_TARGET_DIR`. Parallel workers then contended on Cargo's build lock:
mutation tests timed out while waiting for another worker rather than because
the mutant hung. The host lane also violated the repository's Docker-only
execution contract.

## Decision

All mutation sweeps run through `just mutants` in the development container.
The shared incremental target is intentionally single-worker. The host-native
lane in ADR-0031 is superseded and removed; its staged mutation strategy and
survivor ratchet remain in force.

## Consequences

Mutation outcomes measure the test suite rather than build-lock contention,
and local results use the same toolchain and runtime as CI. Full sweeps take
longer, so they remain scheduled and scoped per crate.

## Alternatives considered

Giving every parallel worker a separate target directory avoids the lock but
duplicates the full dependency graph per worker, exhausts runner disk, and
discards the incremental reuse that makes successive mutants affordable.

Increasing the timeout hides the lock contention and makes genuine infinite
loops slower to detect.

## References

- [ADR-0031](./0031-mutation-testing-for-assertion-strength.md)
- `just mutants`
- `mutants.toml`
