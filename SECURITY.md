# Security policy

## Reporting a vulnerability

If you discover a security vulnerability in afm — a parser crash on
untrusted input, a memory-safety issue, an HTML-injection bypass, or
anything with exploitative potential — **do not open a public
issue**. Instead:

1. Preferred: open a private report via
   [GitHub Security Advisories](https://github.com/P4suta/afm/security/advisories/new).
   This lets us discuss and patch before disclosure.
2. Alternative: email the maintainer at
   `42543015+P4suta@users.noreply.github.com` with the subject
   `[afm security] <short summary>`.

Please include:

- The shortest input or reproduction steps that trigger the issue.
- The afm version / commit hash and the Rust toolchain version.
- Whether the issue is reachable via untrusted input (e.g. rendering
  user-supplied markdown).
- Your proposed CVSS severity, if you have one in mind.

## Response expectations

- We acknowledge reports within **7 days**.
- Triage, patch, and coordinated disclosure typically complete within
  **30–60 days** for high-severity issues, faster for critical ones.
- Credits (unless you prefer anonymity) are noted in `CHANGELOG.md`
  and the security advisory.

## Scope

In scope:
- Crashes, panics, or non-termination on any UTF-8 or Shift_JIS input
  within 10 MiB.
- HTML-escape bypass in the renderer (`afm-parser/src/aozora/html.rs`),
  since rendered output is embedded in web pages.
- CommonMark / GFM conformance regressions that enable a bypass.
- Integer overflow, out-of-bounds reads (we `#![forbid(unsafe_code)]`
  in our own crates; `upstream/comrak/` is unsafe-free too).

Out of scope:
- Bugs in vendored `upstream/comrak/` that also reproduce against
  pristine comrak at the same tag — please report those upstream at
  <https://github.com/kivikakk/comrak>.
- Denial-of-service via inputs that simply take a long time to parse
  without panicking (we track these as perf issues, not vulns).
- Issues in dependencies with no afm-specific exploitation path —
  cargo-deny's advisory check catches these at CI time.

## Supported versions

afm is pre-1.0. Only the `main` branch is supported; security fixes
land there and in the next tagged release.

| Version | Supported |
|---|---|
| main  | ✅ |
| <1.0  | ❌ (use main) |
