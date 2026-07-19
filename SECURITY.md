# Security policy

## Reporting a vulnerability

If you discover a security vulnerability in aozora — a parser crash on
untrusted input, a memory-safety issue, an HTML-injection bypass in the
renderer, or anything with exploitative potential — **do not open a
public issue**. Instead:

1. Preferred: open a private report via
   [GitHub Security Advisories](https://github.com/P4suta/aozora/security/advisories/new).
   This lets us discuss and patch before disclosure.
2. Alternative: email the maintainer at
   `42543015+P4suta@users.noreply.github.com` with the subject
   `[aozora security] <short summary>`.

Please include:

- The shortest input or reproduction steps that trigger the issue.
- The aozora version / commit hash and the Rust toolchain version.
- Whether the issue is reachable via untrusted input (e.g. rendering
  user-supplied 青空文庫 source).
- Your proposed CVSS severity, if you have one in mind.

## Response expectations

- Reports are acknowledged within **7 days**.
- Triage, patch, and coordinated disclosure typically complete within
  **30–60 days** for high-severity issues, faster for critical ones.
- Credits (unless you prefer anonymity) are noted in the security
  advisory.

## Scope

In scope:

- Crashes, panics, or non-termination on any UTF-8 or Shift_JIS input
  within 10 MiB.
- HTML-escape bypass in the renderer (`crates/aozora`), since
  rendered output is embedded in web pages.
- Out-of-bounds reads, integer overflow, use-after-free, or other
  memory-safety violations. The Rust crates use
  `#![forbid(unsafe_code)]`; `aozora-ffi` and the scan implementation carry
  documented carve-outs and are explicitly in scope.
- WASM / Python / C ABI driver issues that are reachable from a
  well-formed host call.

Out of scope:

- Denial-of-service via inputs that simply take a long time to parse
  without panicking. These are tracked as performance issues.
- Issues in third-party dependencies with no aozora-specific
  exploitation path — `cargo deny` and `cargo audit` catch advisories
  at CI time.

## Release profile: `panic = "abort"`

The release profile builds workspace-wide with `panic = "abort"`. A
panic reached at runtime therefore **aborts the entire host process**
(`SIGABRT`): it does not unwind and cannot be caught with
`std::panic::catch_unwind`. The parser targets a panic-free path on
untrusted input (enforced by the fuzz harnesses and the no-bare-`［＃`
Tier-A invariant), but an embedder must treat any residual panic as a
hard crash of its own process.

This matters most at the binding boundaries — the `aozora-ffi` C ABI,
`aozora-wasm`, and the `aozora-py` PyO3 module all run inside a host
process (a C/C++ program, a browser tab's wasm instance, a Python
interpreter). Under `panic = "abort"` a panic crossing the FFI boundary
aborts rather than unwinding into foreign frames, which is the
memory-safe outcome, but it still tears the host down. **Pre-validate
untrusted input** (cap length — the security scope above is bounded at
10 MiB — and reject inputs you will not render) before calling in, and
isolate rendering of attacker-controlled content in a worker /
subprocess if a single parse must not be able to take the host down.
Report any panic reachable from a well-formed host call as a
vulnerability per the policy above.

## Release & supply-chain integrity

Release credentials follow published standards, not a homegrown scheme:

- **No long-lived publish tokens in the repository.** Where the registry
  supports OIDC Trusted Publishing (crates.io, PyPI, npm) we mint a
  short-lived token at publish time; the OIDC-less marketplace tokens
  (`VSCE_PAT`, `OVSX_PAT`) live as GitHub *Environment* secrets, not
  repository secrets.
- **Approval-gated publishing.** Every credential-bearing job runs in the
  `release` GitHub Environment with required-reviewer approval and a
  deployment branch/tag restriction, so nothing ships — and no secret or
  OIDC token is reachable — without a human in the loop.
- **Build provenance.** Release artefacts carry Sigstore-backed SLSA
  Build L2 provenance, verifiable with `gh attestation verify <artefact>
  --repo P4suta/aozora`.
- **Continuous self-assessment.** An [OpenSSF Scorecard](https://scorecard.dev/viewer/?uri=github.com/P4suta/aozora)
  workflow tracks the supply-chain posture (least-privilege tokens,
  SHA-pinned actions, dangerous-workflow detection) and reports to code
  scanning.

The operational details are in the
[release secrets runbook](https://github.com/P4suta/aozora/blob/main/docs/contrib/releasing-secrets.md)
and [ADR-0020](https://github.com/P4suta/aozora/blob/main/docs/adr/0020-release-secret-hardening-trusted-publishing.md).

## Supported versions

aozora is pre-1.0. Only the `main` branch is supported; security fixes
land there and in the next tagged release.

| Version | Supported |
|---|---|
| main  | ✅ |
| <1.0  | ❌ (use main) |
