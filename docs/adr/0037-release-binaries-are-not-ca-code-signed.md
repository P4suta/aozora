# ADR-0037: Release binaries are not CA code-signed

- Status: accepted
- Date: 2026-07-15
- Deciders: @P4suta

## Context

`release.yml` ships an `aozora` binary for three targets, two of which have
an OS-level execution-trust check: Windows SmartScreen and macOS Gatekeeper.
Both raise an "unknown publisher" prompt for a binary that carries no CA
signature. Suppressing that prompt requires Authenticode on the `.exe` and
an Apple Developer ID plus notarization on the macOS build.

The rationale for our answer lived in `docs/contrib/release.md`, a runbook —
so a decision, its rejected alternatives, and the conditions for revisiting
it were filed as a procedure. This ADR is that content's real home; the
runbook now links here.

## Decision

Release binaries are **not** CA code-signed pre-1.0.

What ships instead:

- **Build provenance attestation** (`actions/attest-build-provenance`), on
  every archive since the first tagged release. A Sigstore-backed SLSA
  provenance statement, verified with
  `gh attestation verify <archive> --repo P4suta/aozora`.
- **SHA256SUMS** for integrity, and signed git tags and commits for
  authorship.

## Rationale

Attestation and code signing are not substitutes for one another, and it
matters which problem we actually have. Provenance proves *which CI built
which artefact from which source* — a supply-chain control. CA signing
suppresses a *first-run prompt for someone who double-clicks a download* —
an execution-trust signal. We ship the first because the supply chain is a
real threat surface for a parser. We skip the second because our audience
installs through `cargo install` and package managers, where the prompt
never appears.

The cost is not nominal. HSM-stored keys have been mandatory since
2023-06, and certificate validity has been capped at 458 days since
2026-03, so this is recurring operational overhead, not a one-time
purchase.

### Rejected: a paid CA (ssl.com eSigner and equivalents)

Covers Windows only — third-party CA certificates do **not** satisfy
Gatekeeper, which honours only an Apple-issued Developer ID. It also no
longer buys what it used to: since the 2024 SmartScreen change, EV
certificates no longer confer instant trust, and both OV and EV must build
reputation organically over downloads. So it is a yearly cost that removes
the warning on neither platform on day one.

## Consequences

Users who download an archive directly from the GitHub Release see an
unknown-publisher prompt on Windows and macOS. This is accepted for a
developer CLI.

Revisit post-1.0, and only if desktop double-click installs become a real
distribution path. The paths then are
[SignPath Foundation](https://signpath.io/solutions/open-source-community)
for Windows (free for OSS, Sectigo-issued, HSM-backed, CI-integrated) and
the Apple Developer Program ($99/yr) plus notarization for macOS.

Nothing in the tree enforces this decision, because there is nothing to
enforce: it is the absence of a signing step. If a signing step is ever
added, this ADR is superseded rather than edited.
