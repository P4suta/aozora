# Release process

aozora releases are git-tag-driven: push an annotated `v<semver>`
tag, and `.github/workflows/release.yml` builds the cross-platform
binaries, generates release notes from Conventional Commits, and
publishes the GitHub Release.

## Cutting a release

```sh
# 1. Pre-flight (everything green locally)
just ci                          # lint + build + test + prop + deny + audit + udeps + coverage + book-build
just prop-deep                   # 4096 cases per proptest block
AOZORA_CORPUS_ROOT=… just corpus-sweep

# 2. Bump workspace version
cargo set-version --workspace 0.2.7
git commit -am "chore(release): bump workspace to v0.2.7"

# 3. Refresh CHANGELOG (Unreleased → version)
just changelog                   # runs git-cliff with --unreleased --prepend
git add CHANGELOG.md && git commit -m "docs: refresh CHANGELOG for v0.2.7"

# 4. Tag (annotated)
git tag -a v0.2.7 -m "v0.2.7"
git push origin main v0.2.7
```

`release.yml` reacts to the tag: builds release binaries on three
runners (linux x86_64, macOS arm64, windows x86_64), assembles
tarballs / zips with the `aozora` binary + `LICENSE-MIT` +
`LICENSE-APACHE` + `NOTICE` + `README.md`, and publishes the
archives plus `SHA256SUMS` to the GitHub Release.

## Sanity check after release

```sh
# Verify checksums
curl -L -O https://github.com/P4suta/aozora/releases/download/v0.2.7/SHA256SUMS
curl -L -O https://github.com/P4suta/aozora/releases/download/v0.2.7/aozora-v0.2.7-x86_64-unknown-linux-gnu.tar.gz
sha256sum --check SHA256SUMS

# Verify the binary
tar -xzf aozora-v0.2.7-*.tar.gz
./aozora --version              # prints "aozora 0.2.7"
```

## Why annotated tags?

`git tag -a` creates a tagged-tag object with a message; `git tag`
alone creates a lightweight tag (a bare ref). git-cliff's release
note extraction only walks annotated tags, and the standard
ecosystem expectation (cargo-release, cargo-dist) is that release
tags are annotated. Using lightweight tags would silently break the
changelog generator.

## Why git-tag-driven, not branch-driven?

A `release/v0.2.7` branch model is the alternative. We don't use
it because:

- Single-author workflow doesn't benefit from the parallel-tracks
  model that branch-driven releases enable.
- An annotated tag *is* the release artefact — anything you need to
  retroactively understand about a release lives in `git show v0.2.7`.
  A branch loses that locality.
- Rollback is `git tag -d` + delete the GitHub release. Trivial.

## CHANGELOG generation

[`git-cliff`](https://git-cliff.org/) consumes Conventional Commits
and produces Keep-a-Changelog formatted output:

```sh
just changelog          # incremental: --unreleased --prepend CHANGELOG.md
just changelog-full     # rebuild from scratch
```

`cliff.toml` configures the grouping:

| Commit type | Section in CHANGELOG |
|---|---|
| `feat:` | Added |
| `fix:` | Fixed |
| `perf:` | Performance |
| `refactor:` | Changed |
| `docs:` | Documentation |
| `test:` | Tests |
| `build:` | Build |
| `ci:` | CI |
| `chore:` | (skipped unless scope is `release`) |
| `revert:` | Reverted |

Non-conventional commits are silently skipped (they survive in
`git log` but don't pollute the changelog).

**Why `--unreleased --prepend` over `-o CHANGELOG.md`:**

The full-rebuild form (`-o`) regenerates the entire changelog from
git history every time, which churns the diff for past releases
even when nothing about them changed (whitespace, footer
formatting). The incremental form only writes the new "Unreleased"
section between the latest release and HEAD, leaving past entries
byte-stable.

## Why three release targets and not five?

The CI matrix builds:

- `x86_64-unknown-linux-gnu` (linux x86_64)
- `aarch64-apple-darwin` (macOS arm64)
- `x86_64-pc-windows-msvc` (windows x86_64)

We *don't* build `x86_64-apple-darwin` (macOS Intel — Apple
deprecated the platform; arm64 covers all current Apple Silicon
machines) or `aarch64-unknown-linux-gnu` (linux arm64 — covered by
`cargo install` from source for the niche ARM Linux deployment
case).

Adding a target is one line in `release.yml`; we add them when a
real consumer asks for a binary build of one. Pre-emptive coverage
isn't worth the CI minutes.

## Why not `cargo-dist` / `release-plz`?

Both are mainstream choices; we use a hand-written `release.yml`
because:

- `cargo-dist` is opinionated about archive layout (assumes you ship
  `bin/` + `share/`); aozora's archive is flat (`aozora` +
  `LICENSE-*` + `NOTICE` + `README.md`).
- `release-plz` automates the version-bump + PR flow; for a single-
  author repo the manual `cargo set-version` + `git tag` is two
  commands and one fewer integration to debug.

When the workspace grows past three release targets or aozora
goes multi-author, both will be worth re-evaluating.

## Pre-1.0 SemVer

aozora is currently in the `0.x` series. The contract:

- `0.x.y` → `0.x.y+1`: patches and additions, no breaks. Always safe
  to upgrade.
- `0.x.y` → `0.x+1.0`: may break the API. `cargo-semver-checks`
  flags the breaks during CI; the version-bump commit references the
  break in its body.
- `0.x.y` → `1.0.0`: the API freeze. Post-1.0, breaking changes
  collect on a `next` branch and ship in a major bump.

The MSRV pin (`rust-toolchain.toml`) advances on its own cadence,
roughly quarterly. MSRV bumps are *not* breaking under our pre-1.0
contract — consumers that need a frozen MSRV pin a release tag.

## Publishing to crates.io

Deferred until v1.0. The reasoning:

- Pre-1.0 every minor bump may break the API; pushing those churns
  the registry for downstream `Cargo.lock` consumers.
- Once published, the published name becomes load-bearing — name
  changes cost goodwill. Holding the name *unpublished* keeps the
  option to refactor the crate boundary.

When v1.0 lands, the publication workflow will run from a tag:
`cargo publish` per crate in topological order
(`aozora-spec` first, `aozora` last), driven from `release.yml`.

## Code signing

Release binaries are **not** CA code-signed (no Authenticode on the
Windows `.exe`, no Apple Developer ID / notarization on the macOS
build). This is a deliberate pre-1.0 decision.

What we ship instead — and why it covers the current audience:

- **Build provenance attestation** (`actions/attest-build-provenance`,
  since v0.4.0): every archive carries a Sigstore-backed SLSA
  provenance statement, verifiable with `gh attestation verify
  <archive> --repo P4suta/aozora` — no certificates, no CA. It proves
  *which CI built which artefact from which source*: a supply-chain
  control, **not** an OS-level execution-trust signal.
- **SHA256SUMS** for integrity; **signed git tags / commits** for
  authorship.

CA code signing solves a *different* problem — suppressing the Windows
SmartScreen / macOS Gatekeeper "unknown publisher" prompt for end users
who double-click a downloaded binary. For a parser library + developer
CLI installed via `cargo install` / package managers, that prompt is
low-friction, so the recurring cost and operational overhead (HSM-stored
keys mandatory since 2023-06; ≤458-day cert validity since 2026-03) is
not justified yet.

When we revisit this (post-1.0, if desktop double-click installs become
a real distribution path):

- **Windows** → [SignPath Foundation](https://signpath.io/solutions/open-source-community)
  free OSS code signing (Sectigo-issued, HSM-backed, CI-integrated).
  Note the 2024 SmartScreen change: EV no longer buys *instant* trust —
  both OV and EV build reputation organically over downloads.
- **macOS** → Apple Developer ID ($99/yr Apple Developer Program) +
  notarization. Third-party CA certs (e.g. ssl.com) do **not** satisfy
  Gatekeeper; only an Apple-issued Developer ID does.
- A paid CA (ssl.com eSigner, etc.) was evaluated and rejected: it
  covers Windows only, no longer removes the first-run warning on day
  one, and adds a yearly cost the project does not need pre-1.0.

## See also

- [Development loop](dev.md) — the local pre-flight commands.
- [Testing strategy](testing.md) — `prop-deep` and corpus sweep
  details.
