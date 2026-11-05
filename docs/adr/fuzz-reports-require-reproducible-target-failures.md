---
date: 2026-10-24
status: proposed
---

# Fuzz reports require reproducible target failures

## Context

A failed verification process doesn't establish a defect in the program under test.
The fuzz workflow treated every cargo-fuzz error as a parser crash, including a missing manifest before the engine started.
The artifact upload accepted an empty directory, so the resulting issue promised an input that didn't exist.
The triage recipe also discarded command failures while interpreting selected text in the log.

[cargo-fuzz 0.13.2](https://github.com/rust-fuzz/cargo-fuzz/blob/0.13.2/src/project.rs) reports both build errors and unsuccessful engine runs through its error return.
An outer cargo-fuzz exit code alone doesn't identify the engine's failure mode.

## Decision

The Rust xtask owns fuzz execution and its reporting decision.
It checks whether the selected package manifest declares the target and reads the dated nightly from the project tool configuration.
The build explicitly selects the package with `--fuzz-dir .` and an isolated target directory.
A failed build stops before the engine runs.

After a successful build, the xtask executes the produced engine directly with distinct exit codes for an input failure and a timeout.
Each soak receives a new artifact directory, so prior inputs can't support a new report.
A crash report requires one crash or leak input and an immediate replay with the same executable and input bytes.
Artifacts from memory exhaustion, timeouts, and unknown failures leave verification incomplete.
The xtask hashes the executable and input around replay and refuses changed identities.
It retains the completed process statuses and identities with the input.

An unsupported exit, missing input, interrupted process, ambiguous artifact set, or unsuccessful replay leaves verification incomplete.
The native command fails for both a reproduced input failure and an incomplete result.
The workflow uploads retained evidence, opens an issue only for a reproduced input failure, and fails whenever the verification step didn't succeed.
An upload failure prevents an issue from promising an unavailable artifact.

Triage uses the same engine build and explicit exit semantics.
Missing inputs and process errors can't establish that historical inputs now pass.

## Verification

The classification proof admits every optional process exit code, artifact count, optional replay exit code, and filename of at most 255 bytes under the Linux target semantics.
Its source-bound Kani obligation must establish the acceptance conditions and reach all three outcomes.
Rejecting controls cover startup errors, missing inputs, successful replays, replay startup errors, and interruption.
The release gate uses the existing pinned proof runner and validates the complete inventory and result statuses.

Adapter tests exercise build failure, stale inputs, ambiguous results, exact replay, changed bytes, target validation, and triage failures through the process boundary.
Real engine controls must confirm the pinned libFuzzer exit and artifact behavior.
The formal result concerns the classification under its declared input interpretation.
Actual compilation, operating-system process behavior, artifact transport, and GitHub permissions require their own execution evidence.

## Consequences

The reporter needs both the stable Rust xtask toolchain and the pinned nightly engine toolchain.
Unclassified process failures remain visible as failing jobs with incomplete evidence.
The reporter doesn't open a parser defect issue until the input reproduces.
