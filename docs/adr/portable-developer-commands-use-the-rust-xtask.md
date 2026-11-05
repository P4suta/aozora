---
date: 2026-10-24
---

# Portable developer commands use the Rust xtask

Status: proposed.

## Context

The setup and maintenance recipes rely on Linux shell utilities or Bash features absent from macOS Bash 3.2.
Lefthook installation also conflicts with an existing global hook dispatcher.
[ADR-0053](0053-locked-native-mise-replaces-docker.md) makes the locked native environment the development authority.
The developer commands must preserve that authority on macOS and Linux.

## Decision

Keep Justfile recipes as argument-preserving calls into the existing Rust xtask.
Use the declared mise tool set directly instead of parsing its text representation in shell.
Run the native prerequisite script before compiling the xtask during setup.
The script performs Linux checks on Linux and skips those checks on macOS.
The small shell adapter remains available before provisioning Rust and native compiler dependencies.

Parse dependency markers as canonical whole-second UTC timestamps.
Reject missing, malformed, future, and older-than-seven-day evidence.
The pure rule returns the decision and exact elapsed seconds.
The command converts elapsed seconds to days for its output.

Create proposed decision records with descriptive filenames and a UTC front-matter date.
Publish through an atomic operation that refuses an existing destination.
Preserve every existing decision record.

Preserve a configured `core.hooksPath` dispatcher and validate the project Lefthook configuration.
Install or remove local Lefthook stubs only when no dispatcher path exists.
Keep the same project and CI gates.

The release quality gate uses the pinned Kani verifier to check the production freshness rule.
Its inventory includes the admission harness and rejecting controls.
The gate checks exact harness identities, complete reports, reachable outcomes, assertion-specific control failures, and unchanged source bytes.
Receipts bind source, verifier files, and configuration.
Native integration tests cover parsing, filesystem publication, Git configuration, and command arguments outside the pure proof model.

## Consequences

Developer commands no longer require `date -d`, `sed -i`, or Bash `mapfile`.
Existing hook dispatchers keep control of Git hook execution.
Missing or incomplete proof evidence prevents release verification.
Formal results establish the declared pure timestamp contract under the pinned Rust and verifier semantics.
They don't establish filesystem, clock, process, or Git service behavior, which require native boundary checks.

## Alternatives considered

Separate Linux and macOS shell recipes would duplicate parsing and state transitions.
Replacing the global dispatcher would change unrelated repositories and user configuration.
Treating an expired or future marker as fresh would hide missing dependency evidence.

## References

[Issue #685](https://github.com/P4suta/aozora/issues/685) defines the affected developer commands.
The [Rust xtask implementation](../../crates/aozora-xtask/src/developer.rs) owns the process and filesystem boundaries.
The [freshness rules](../../crates/aozora-xtask/src/developer/rules.rs) contain the production decision and its proofs.
