# 0054. Native performance baseline epoch

- Status: accepted
- Date: 2026-07-30
- Deciders: @P4suta
- Tags: performance, ci, dx

## Context

ADR-0053 moved the performance gate from the Debian development image to the
native Ubuntu CI environment. Callgrind instruction counts are stable within a
fixed compiler, linker, ABI, and profiler environment, but the absolute counts
are not portable across those environments.

A clean `origin/main` build reproduced the short parse and render ceiling
overages under the native environment. The application changes in ADR-0053
were therefore not a performance regression, while continuing to use the
Docker-calibrated ceilings would block the native gate.

## Decision

The Docker-calibrated performance baseline remains frozen as
`perf-baseline.tsv`. The active baseline is
`perf-baseline-native-v1.tsv`, calibrated with the locked Rust toolchain and
Ubuntu's Valgrind. Both files remain under the monotonic baseline ratchet, and
the performance runner and ratchet share the active path in code.

A measurement-environment change starts a new named baseline epoch only after
the unchanged base revision reproduces the difference. Existing ceilings that
still pass remain unchanged in the new epoch.

## Consequences

Performance regressions remain merge-blocking within the native epoch. The
legacy baseline preserves the comparison boundary without forcing native
development to reproduce the retired linker and image.

Future compiler, linker, ABI, or profiler migrations may require another
explicit epoch rather than weakening an existing baseline.
