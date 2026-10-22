---
date: 2026-10-23
---

# Dependency license policy

## Status

Proposed.

## Context

Cargo and GitHub dependency review interpret license metadata differently.
Their exceptions need reasons that remain reviewable outside the configuration.

## Decision

Keep the accepted license lists in [deny.toml](../../deny.toml) and [dependency review](../../.github/workflows/ci.yml) explicit.
`MIT-0` permits reuse without attribution and is compatible with the workspace licenses.
`BSL-1.0` permits redistribution and covers rustyline's Windows clipboard dependencies.
Keep Crossterm-only features to exclude termwiz and its `WTFPL` dependency.

GitHub dependency review exempts Swatinem/rust-cache because it runs only in CI and isn't distributed with the crates.
It exempts protobuf because the license scanner doesn't recognize its permissive Go patent grant as an SPDX license.
It exempts const-siphasher and filetime because cargo-deny accepts their legacy `MIT/Apache-2.0` metadata.
Cargo-deny still checks dependency licenses.

Keep dependency review's advisory threshold at moderate and cargo-deny's advisory checks enabled.
The configuration ignores no RustSec advisories.
Path-only internal dependencies follow [ADR-0017](0017-ecosystem-dependency-pin-policy.md).

## Consequences

Dependency review can accept the documented metadata differences without accepting unknown licenses.
New exceptions require a reason and an update to this decision.

Rejecting every metadata difference would reject these dependencies despite their accepted licenses.
Disabling license or advisory checks would hide unrelated dependency risks.
