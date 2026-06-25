# 0020. Release secret hardening via Trusted Publishing and environment gates

- Status: accepted
- Date: 2026-06-26
- Deciders: @P4suta
- Tags: release, security, ci, supply-chain

## Context

The release workflows already met most of the industry baseline:
least-privilege `permissions`, full-SHA-pinned actions, `concurrency`
guards, `dry_run: true` defaults, PyPI OIDC, and Sigstore build-provenance
attestations (SLSA Build L2). One gap remained, and it was the important
one: the credentials that actually publish releases — `CARGO_TOKEN`,
`NPM_TOKEN`, `VSCE_PAT`, `OVSX_PAT` — lived as **repository secrets with
no approval gate**. Any workflow run reachable by anyone with write access
(a `workflow_dispatch`, a pushed tag, a compromised third-party action)
could read them, and a pushed `vscode-v*` tag published to the VS Code
Marketplace with no human in the loop.

We want to follow the **published standards** here, not a homegrown
scheme: GitHub Actions security hardening, the Trusted Publishing (OIDC)
flows that crates.io / PyPI / npm now offer, and OpenSSF / SLSA guidance.

## Decision

1. **Gate every credential-bearing job behind a dedicated `release`
   GitHub Environment.** Each publish/release workflow splits into an
   ungated `build`/`package` job (no credentials) and a gated `publish`
   job with `environment: release`. The environment requires reviewer
   approval and restricts deployments to `main` + `v*` / `vscode-v*`
   refs. Per GitHub's docs, environment secrets and the OIDC token are
   unreachable until a protection rule passes.

2. **Eliminate long-lived publish tokens wherever OIDC exists.**
   - crates.io → `rust-lang/crates-io-auth-action` mints a 30-minute,
     auto-revoked token (`use_oidc` input, default true).
   - PyPI → existing `pypa/gh-action-pypi-publish` OIDC, now scoped to
     the `release` environment.
   - npm → `setup-node` + `id-token: write`; npm attaches provenance
     automatically.

3. **Keep the OIDC-less tokens as environment secrets, not repo
   secrets.** VS Code Marketplace (`VSCE_PAT`) and Open VSX (`OVSX_PAT`)
   have no OIDC, so their tokens move to the `release` environment and the
   repository-level copies are deleted.

4. **Handle the registry bootstrap honestly.** crates.io and npm do not
   allow Trusted Publishing for a package that does not exist yet, so the
   first publish uses a one-time environment-scoped token (`use_oidc=false`),
   after which the trusted publisher is registered and the token deleted.
   PyPI needs no bootstrap (pending publisher). This is documented in the
   [release secrets runbook](../../crates/aozora-book/src/contrib/releasing-secrets.md).

5. **Add OpenSSF Scorecard** (`ossf/scorecard-action`) as a continuous,
   standards-based self-assessment so the hardening posture cannot
   silently regress.

## Consequences

- A pushed release tag or a `dry_run=false` dispatch now pauses for
  maintainer approval before any artefact ships — the desired control for
  a project where the maintainer is the sole publisher.
- Steady-state releases store **zero** long-lived registry tokens; the
  only remaining secrets are the two marketplace PATs, behind the gate.
- Dry-runs stay frictionless: the ungated build/package jobs need no
  approval, so smoke tests are unchanged.
- Slightly more workflow surface (job splits, artefact hand-off for
  npm/extism) and a one-time per-crate trusted-publisher registration.
- We stay at SLSA Build L2. Build L3 would require moving the build into
  a reusable workflow to isolate it from user-controlled steps; that is
  disproportionate for this project and deferred.

## Alternatives considered

- **Environment-gate the existing tokens without adopting OIDC.** Simpler
  (no registry-side trusted-publisher setup), but leaves long-lived
  `CARGO_TOKEN` / `NPM_TOKEN` in storage. Rejected: OIDC removes the
  secret entirely, which is the stronger, standard-recommended posture.
- **Copy P4suta/find-my-files' bespoke signing pipeline.** Useful as
  prior art, but it is tailored to Windows Authenticode and SSL.com
  eSigner. We deliberately ground this work in vendor-neutral official
  documentation instead.
- **Pursue SLSA Build L3 now (reusable workflows).** Over-engineered for
  a pre-1.0 parser; L2 + environment approval is the standard-sufficient
  level here.

## References

- Plan: `~/.claude/plans/secret-find-my-files-http-github-com-p4-atomic-cupcake.md`
- Runbook: [Release secrets & Trusted Publishing](../../crates/aozora-book/src/contrib/releasing-secrets.md)
- GitHub — Security hardening for Actions; Managing environments; OIDC hardening
- crates.io / PyPI / npm Trusted Publishing official docs
- OpenSSF Scorecard; SLSA v1.0 levels
- Prior art: [P4suta/find-my-files](https://github.com/P4suta/find-my-files)
