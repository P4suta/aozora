# Release process

Releases are Release-PR driven by [release-plz](https://release-plz.dev/).
Conventional Commits land on `main`; release-plz keeps one Release PR open
that bumps `[workspace.package].version` and rewrites `CHANGELOG.md`.
Squash-merging it publishes every public crate to crates.io and cuts a
single `vX.Y.Z` tag, which fans out to `release.yml` and the tag-driven
PyPI / npm / Extism publishers.

**Humans never hand-edit a version or hand-push a release tag.**

> release-plz is **dormant until activated** — `release-plz.yml` no-ops
> green until the GitHub App secrets exist. Activation is in the
> [release secrets runbook](releasing-secrets.md).

## Pre-flight

Three obligations that **no gate enforces**. Nothing goes red if you skip
them; that is why they are written down.

- [ ] **`just fuzz-all-deep`** is green — zero crash / leak / oom
      artifacts.
- [ ] **`cross-os` is green from a manual dispatch** on the release commit
      (Actions → cross-os → Run workflow). It runs the suite natively on
      macOS and Windows — the platforms `release.yml` ships binaries for
      but only *cross-builds* on. Local `just` cannot reproduce those
      runners, so this run is the only authority, and the workflow is
      `schedule` + `workflow_dispatch` only.
- [ ] **Mutation baselines hold** — `just mutants -p <crate>` for each
      crate in `mutants-baseline.json` reports no more survivors than its
      committed count. The weekly `mutants` workflow is a mirror, not a
      release gate.

## Cutting a release

1. Land changes on `main`. release-plz opens or updates the Release PR.
2. Review it — the version bump and the rewritten `CHANGELOG.md` section.
3. Add the `release: approved` label, then re-run the `release-gate` job to
   flip `ci-success` green.
4. **Squash**-merge it by hand. Squash rather than rebase: GitHub web-flow
   GPG-signs the single resulting commit, so `main` never receives
   release-plz's unsigned bot commits. (Auto-merge is force-disabled on
   these PRs by `no-automerge-on-release-pr.yml`.)

The merge does the rest unattended: crates.io publish in dependency order
via OIDC, the `vX.Y.Z` tag, then the downstream publishers. Those run in
the reviewer-gated `release` environment — approve the batch once under
Actions → the run → Review deployments.

Afterwards:

```sh
gh attestation verify aozora-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz --repo P4suta/aozora
```

## Pre-1.0 SemVer

- `0.x.y` → `0.x.y+1` — no breaks. Always safe to upgrade.
- `0.x.y` → `0.x+1.0` — may break. `cargo-semver-checks` flags the breaks
  on the Release PR.
- `0.x.y` → `1.0.0` — the API freeze.

An MSRV bump is **not** breaking under this contract, and the six-month
rule ([ADR-0034](../adr/0034-separate-toolchain-channel-from-msrv.md)) is
what makes that claim mean something: any toolchain from the last six
months keeps working, so a bump is predictable rather than a surprise you
can only absorb by pinning a tag. See [MSRV](./msrv.md).

## Why three release targets

`release.yml` builds linux x86_64, macOS arm64, and windows x86_64. Not
macOS Intel (Apple deprecated it; arm64 covers current machines) and not
linux arm64 (`cargo install` from source covers that niche).

Adding one is a line in `release.yml`. We add a target when a real consumer
asks for a binary build of it — pre-emptive coverage is not worth the CI
minutes. ([ADR-0021](../adr/0021-cli-release-stays-hand-written.md) cites
this policy for why `cargo-dist` was not adopted.)

## See also

- [Release secrets & Trusted Publishing](releasing-secrets.md) — the App,
  the environments, and per-registry publisher setup, including activation.
- [ADR-0037](../adr/0037-release-binaries-are-not-ca-code-signed.md) — why
  the binaries are not CA code-signed.
