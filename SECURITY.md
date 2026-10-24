# Security policy

## Reporting a vulnerability

Report security vulnerabilities privately.
Examples include parser crashes on untrusted input, memory-safety violations, and HTML-injection bypasses in the renderer.
Don't open a public issue for a vulnerability.

1. Open a private report through [GitHub Security Advisories](https://github.com/P4suta/aozora/security/advisories/new).
   Private reports support discussion and patching before disclosure.
2. Send a private report by email to `42543015+P4suta@users.noreply.github.com` with the subject `[aozora security] <short summary>` as an alternative.

Please include:

- The shortest input or reproduction steps that trigger the issue.
- The aozora version / commit hash and the Rust toolchain version.
- Whether untrusted input can reach the issue, including user-supplied source text.
- Your proposed Common Vulnerability Scoring System severity, if available.

## Response expectations

- Maintainers acknowledge reports within **7 days**.
- Triage, patching, and coordinated disclosure typically take **30–60 days** for high-severity issues and less time for critical issues.
- Security advisories credit reporters unless they request anonymity.

## Scope

In scope:

- Crashes, panics, or non-termination on UTF-8 or Shift_JIS input within 10 MiB.
- HTML-escape bypasses in `crates/aozora` that affect rendered output embedded in web pages.
- Out-of-bounds reads, integer overflow, use-after-free, or other memory-safety violations.
  Rust crates use `#![forbid(unsafe_code)]`, with documented exceptions in `aozora-ffi` and the scan implementation.
  Both exceptions remain in scope.
- Driver issues in `aozora-wasm`, `aozora-py`, or `aozora-ffi` reachable from a well-formed host call.

Out of scope:

- Inputs that take a long time to parse without panicking.
  Report such inputs as performance issues.
- Third-party dependency issues without an aozora-specific exploitation path.
  CI checks dependency advisories through `cargo deny` and `cargo audit`.

## Release profile: `panic = "abort"`

The workspace release profile uses `panic = "abort"`.
A runtime panic aborts the entire host process with `SIGABRT`.
It doesn't unwind, and `std::panic::catch_unwind` can't catch it.
Fuzz harnesses and the Tier-A invariant for bare `［＃` test the parser's intended panic-free behavior on untrusted input.
An embedder must still treat a residual panic as a crash of its own process.

The `aozora-ffi`, `aozora-wasm`, and `aozora-py` bindings run within their host runtime.
With `panic = "abort"`, a panic aborts execution instead of unwinding into foreign frames.
Validate untrusted input before calling a binding, including a length limit within the 10 MiB security scope.
Use a separate worker or subprocess when a parsing crash mustn't stop the host app.
Report any panic reachable from a well-formed host call as a vulnerability under the reporting policy.

## Release and supply chain integrity

Release credentials and verification use these controls:

- **No long-lived publish tokens in the repository.**
  Registries that support OIDC Trusted Publishing use short-lived publication credentials.
  Marketplace tokens such as `VSCE_PAT` and `OVSX_PAT` belong in protected GitHub environment secrets when the optional marketplace channel needs them.
- **Approval-gated publishing.**
  Publishing and signing jobs use the `release` GitHub environment.
  Release proposals, crates.io release orchestration, and automation credential verification use `release-plz`.
  Both environments require approval by `P4suta` and prevent administrator bypass.
  The `release` environment permits `main` and tags matching `v*` or `vscode-v*`.
  The `release-plz` environment permits only `main`.
- **Build provenance.**
  Release artifacts carry Sigstore-backed SLSA Build L2 provenance.
  Verify it with `gh attestation verify <artifact> --repo P4suta/aozora`.
- **Continuous self-assessment.**
  [OpenSSF Scorecard](https://scorecard.dev/viewer/?uri=github.com/P4suta/aozora) checks token permissions, SHA-pinned actions, and dangerous workflows.
  The workflow reports findings to code scanning.

The [release secrets runbook](https://github.com/P4suta/aozora/blob/main/docs/contrib/releasing-secrets.md) describes credential setup and verification.
[ADR-0020](https://github.com/P4suta/aozora/blob/main/docs/adr/0020-release-secret-hardening-trusted-publishing.md) records the credential design.

## Supported versions

aozora is pre-1.0.
Only the `main` branch receives security fixes, which also ship in the next tagged release.

| Version | Supported |
|---|---|
| main  | ✅ |
| <1.0  | ❌ Use main |
