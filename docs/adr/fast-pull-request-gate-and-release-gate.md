---
date: 2026-10-22
---

# Fast pull request gate and release gate

## Status

proposed

## Context

A pull request waited about 24 minutes for `required` in [run 39763935210](https://github.com/P4suta/aozora/actions/runs/39763935210).
`just ci-rust` alone took 23 minutes, and `lint-full`, the build with its tests, and coverage took 17 of them.
CodeQL for Rust took 11 minutes, the corpus job 10, the web suite 5, and each of eight mutation shards 2 to 4.
Daily and weekly schedules reran the heavy suites on an unchanged `main`.
A merge queue needs every required check to report on the `merge_group` event, and `ci.yml` had no such trigger.

## Decision

Every pull request and every merge group runs a fast deterministic gate, and `required` stays its only summary.
The gate runs `rust` for the build, nextest, and default doctests, `rust-lint`, `rust-contracts` for property, conformance, and drift checks, and the existing fast jobs.
`commitlint`, `release-gate`, and dependency review run only on pull requests.
A merge group holds the commits that GitHub writes, and the release label passed its check before the change entered the queue.

`release-ready` runs every check that left the pull request gate.

| Check | Old job | Release job |
| --- | --- | --- |
| Coverage, all-feature doctests, internal tests, and deep properties | `rust` | `quality` through `ci-rust-release` |
| Corpus gates | `corpus` | `quality` through `ci-corpus` |
| Callgrind performance gate | `performance` | `quality` through `ci-perf` |
| Linux arm64 tests | `arm64` | `arm64` |
| Mutation testing | `mutants-in-diff` | `mutation`, a full sweep |
| Fuzz builds | `fuzz-build` | `fuzz` |

The baseline ratchet moves to the `release` job, which still refuses a pull request that loosens a baseline.
The nightly, performance, mutation, fuzz, and cross-OS workflows keep only manual dispatch.
The weekly specification drift check and Scorecard keep their schedules.
`just ci` mirrors the fast gate on Linux, and other hosts run `just ci-repo` before a push.

This record partly supersedes the pull request graph of ADR-0051 and the merge-blocking performance gate of ADR-0054.
It extends [merges gated by the required check](merges-gated-by-the-required-check.md).

## Consequences

The estimate for the pull request gate is 12 to 14 minutes, with CodeQL for Rust on the critical path, until the first hosted runs measure it.
A change that regresses coverage, mutation, corpus, or performance can merge, and `release-ready` then blocks its publication until a fix lands.
Contributors can run the heavy recipes on demand when a change needs that evidence early.
