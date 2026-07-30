# 0053. Locked native mise replaces Docker

- Status: accepted
- Date: 2026-07-30
- Deciders: @P4suta
- Tags: infra, ci, release, dx

## Context

The development image duplicated version authorities across a Dockerfile,
Compose, host mise configuration, and workflow setup actions. Most repository
gates exercised compilers and command-line tools and did not need container
isolation. The image also forced corpus path translation, generated root-owned
artifacts, and required custom workflow parsers and a hand-maintained parallel
lane to prove that its wrappers remained connected.

Removing the container must not remove the behavioral evidence required by
[ADR-0042](0042-release-ready-is-the-publish-authority.md), the release fan-in
from [ADR-0051](0051-ci-and-release-ready-split-event-driven-release-fan-in-and-the-actions-concurrency-budget.md),
or the code-proof reuse from
[ADR-0052](0052-code-gate-reuse-by-code-identity.md).

## Decision

The supported development environment is native and provisioned by
`mise.toml` plus `mise.lock`. `rust-toolchain.toml` declares the stable Rust
toolchain, components, and targets; the corresponding mise entry is checked
against it by `xtask msrv check`. Operating-system libraries such as Clang and
libclang, Valgrind, and Playwright dependencies remain explicit host
prerequisites.

`just` recipes invoke tools directly. GitHub Actions installs locked subsets
with mise and calls fixed `ci-*` recipes. Every ordinary pull request runs the
same suites; only release commits retain the event-driven deferral to
`release-ready`. Mutation, performance, corpus, WASM, Python, and native arm64
evidence remain required.

Dockerfiles, Compose, the development container, the development-image
workflow, `cross`, and Docker-specific local CI emulation are removed.
Compiler lints and executable build or test commands replace text parsers that
only checked repository configuration wiring. The source-coordinate lint is
retained because it enforces a repository contract that the compiler cannot
express.

Mutation keeps a dedicated incremental target and one worker per shard.
arm64 runs on a native runner. musl artifacts build with the native
architecture's musl toolchain.

This supersedes the Docker execution decisions in ADR-0007, ADR-0031, and
ADR-0040. Their testing intent and recorded measurements remain historical
context.

## Consequences

Developers and CI exercise the same commands without filesystem or UID
translation. Tool updates change one manifest and its checksum lockfile.
Platform-specific prerequisites are visible and must be installed explicitly.

The release-ready qualify, code-proof, quality, mutation, fuzz, sanitizer,
cross-OS, artifact, Python, and fan-in structure is unchanged. Artifacts are
still rebuilt and exercised for every qualified release commit.

## Alternatives considered

A staged Docker fallback would retain two supported environments and recreate
the drift this decision removes. Keeping Docker only for mutation or
cross-compilation would preserve the shared build-lock and emulation problems
while native runners already provide the required architectures.
