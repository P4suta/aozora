---
date: 2026-10-23
---

# Repository lint exceptions

## Status

Proposed.
Extends [ADR-0004](0004-lint-profile-policy.md) and [item-level lint expectations](item-level-lint-expectations.md).

## Context

Workspace lint configuration needs exceptions for API naming, dependency resolution, and test diagnostics.
Their reasons belong in one record rather than beside individual settings.

## Decision

Keep the following configuration in [Cargo.toml](../../Cargo.toml) and [clippy.toml](../../clippy.toml).

- Accept module name repetition so public types remain clear when imported on their own.
- Accept different crate versions because transitive dependencies can't always share one version.
- Accept missing const qualifiers because a public function becomes const only when its API commits to const evaluation.
- Accept explicit crate visibility inside private modules to keep internal API boundaries clear.
- Accept the listed domain terms and encoding names in documentation without requiring code formatting.
- Keep the type complexity threshold at 250 because lower thresholds flag parser type nesting without improving readability.
- Accept unwrap, expect, and panic in tests so assertions can report failures directly.
- Permit breaking exported API suggestions while the API remains before version 1.0.

Keep the [Justfile](../../Justfile) test, example, and benchmark Clippy exceptions for panics, unwrap, expect, and diagnostic output.
Production library and binary checks keep their warning-denying configuration.
The semver check excludes tree-sitter-aozora because it has no published baseline.
Inline parser payloads follow [ADR-0036](0036-index-owned-ast-replaces-the-arena.md).
MSRV checks follow [ADR-0034](0034-separate-toolchain-channel-from-msrv.md).
New item-level exceptions must use fulfilled expectations with a reason.

## Consequences

The existing lint surface remains unchanged and its exceptions have a reviewable rationale.
Removing every exception would create diagnostics for these accepted API and test conventions.
Adding broader exceptions would weaken checks outside their stated purpose.
