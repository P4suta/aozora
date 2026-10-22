---
date: 2026-10-22
---

# Item-level lint expectations

## Status

Proposed.
Partly supersedes [ADR-0004](0004-lint-profile-policy.md), only its `#[allow(lint, reason = "…")]` rule.

## Context

An `allow` attribute keeps suppressing a lint after its original cause disappears.
An `expect` attribute reports an unfulfilled expectation when the lint stops firing.
Exceptions must therefore remain attached to the item that requires them.

Cargo 1.97.0 rejects a crate `[lints]` table combining `workspace = true` with local overrides with `cannot override workspace.lints in lints`.
[Clippy 1.97.0 documents that `allow_attributes` checks only outer `#[allow]` attributes](https://clippy.rust-lang.org/rust-1.97.0/index.html#allow_attributes), leaving inner `#![allow]` attributes unchecked.
The remaining crate-wide exceptions in `aozora-trace` cover findings across multiple modules and require a separate change.

## Decision

Fix the code a lint names.
Use a narrow item-level `#[expect(lint, reason = "…")]` only where a macro, an ABI, or generated code forces an exception.
Keep unfulfilled expectations visible to the warning-denying lint gate.
Do not add inner crate-wide or file-wide suppression.

Resolve crate-wide differences in code rather than combining inherited workspace lints with a crate-local override table, which Cargo rejects.
`aozora-ffi` retains its existing separate lint table with `unsafe_code = "deny"` and expects unsafe code only on individual items required by its C ABI.

Enable `clippy::allow_attributes` in the workspace restriction lint profile to refuse new outer `allow` attributes.
Remove remaining inner attributes in later changes, then add a repository check that refuses them too.
Leave `aozora-trace/src/` for a later change together with its other findings.

## Consequences

Unnecessary exceptions fail the lint gate instead of silently surviving refactors.
Code fixes remain the default, and unavoidable exceptions carry their reason at the affected item.
Generated expectations must be fulfilled in every build that compiles them, using `cfg_attr` when a lint fires only outside tests.
Clippy alone cannot enforce the ban on inner suppression until the remaining attributes are removed and the repository check is added.
