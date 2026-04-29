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
- HTML-escape bypass in the renderer (`crates/aozora-render`), since
  rendered output is embedded in web pages.
- Out-of-bounds reads, integer overflow, use-after-free, or other
  memory-safety violations. The Rust crates use
  `#![forbid(unsafe_code)]`; `aozora-ffi` and `aozora-scan` carry
  documented carve-outs and are explicitly in scope.
- WASM / Python / C ABI driver issues that are reachable from a
  well-formed host call.

Out of scope:

- Denial-of-service via inputs that simply take a long time to parse
  without panicking. These are tracked as performance issues.
- Issues in third-party dependencies with no aozora-specific
  exploitation path — `cargo deny` and `cargo audit` catch advisories
  at CI time.

## Supported versions

aozora is pre-1.0. Only the `main` branch is supported; security fixes
land there and in the next tagged release.

| Version | Supported |
|---|---|
| main  | ✅ |
| <1.0  | ❌ (use main) |
