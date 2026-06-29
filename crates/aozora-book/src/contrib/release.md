# Release process

aozora releases are git-tag-driven: push an annotated `v<semver>`
tag, and `.github/workflows/release.yml` builds the cross-platform
binaries, generates release notes from Conventional Commits, and
publishes the GitHub Release.

## Cutting a release

```sh
# 1. Pre-flight (everything green locally)
just ci                          # lint + build + test + prop + deny + audit + shear + coverage + book-build
just prop-deep                   # 4096 cases per proptest block
AOZORA_CORPUS_ROOT=… just corpus-sweep
just smoke-py                    # host-side: abi3 wheel build + mypy + pytest (not in `just ci`)

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

## Build channels (dev / nightly / stable)

Every `aozora` / `aozora-lsp` binary reports a channel-aware version, so a
build's origin is unambiguous — a local build can never be mistaken for a
release. The string is resolved at build time by the `aozora-buildstamp` crate;
its format has a single source, the `xtask version` subcommand:

| Channel   | Example                            | Where it comes from                              |
| --------- | ---------------------------------- | ------------------------------------------------ |
| `dev`     | `0.4.1-dev+g3672e3f` (`.dirty`)    | any local `cargo build` (a workspace `.git`)     |
| `nightly` | `0.4.1-nightly.20260630+g3672e3f`  | `.github/workflows/nightly.yml` (tip of main)    |
| `stable`  | `0.4.1`                            | `.github/workflows/release.yml` / a crates.io install |

SemVer pre-release ordering gives `…-dev < …-nightly.<date> < X.Y.Z`, so a
non-release build always sorts below the release it heads toward. A crates.io
install reports the clean triple (no `.git`, no override → stable); only a
working-copy build carries `-dev`. The format comes from one place:

```sh
cargo run -p aozora-xtask -- version --channel nightly --date 20260630
cargo run -p aozora-xtask -- version --channel stable      # → 0.4.1
```

### Nightly builds

`.github/workflows/nightly.yml` builds an **unsigned** `aozora` CLI from the tip
of main daily (and on demand), stamped `X.Y.Z-nightly.<date>+g<sha>`. It is
published as a **14-day GitHub Actions artifact** — not a Release — so it stays
off the Releases list and needs no tag. Grab the latest:

```sh
gh run download --repo P4suta/aozora -n aozora-nightly
```

Nightlies are unsigned, Linux x86_64 only, and carry no stability guarantee; the
scheduled run skips when main has not moved in 24h. They are for testing main
between releases — the signed, multi-platform pipeline stays `release.yml`.

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

When you raise the MSRV, bump the **Dockerfile `FROM rust:` base in the
same commit** so the dev image keeps building on exactly the pinned
channel (one toolchain, no dead second one). Dependabot deliberately
ignores the `rust` base image (`.github/dependabot.yml`) precisely so it
cannot drift ahead of `rust-toolchain.toml`, so this base bump is manual.
Resolve the new digest with `docker buildx imagetools inspect
rust:<ver>-bookworm`.

## Publishing to crates.io

Live since v0.4.1. The whole workspace publishes through the manual
`.github/workflows/publish-crates.yml` workflow:

```sh
gh workflow run publish-crates.yml -f dry_run=false   # then approve the gated publish job
```

An ungated `package` job dry-runs the dependency-free leaf crates as a
metadata smoke test on every dispatch. The live `publish` job runs in the
`release` GitHub Environment (required-reviewer approval) and walks an
**explicit topological ladder** — `aozora-encoding` / `aozora-spec`
first, `aozora` and `aozora-cli` last — waiting for crates.io index
propagation between dependent crates. (We use the explicit ladder, *not*
`cargo publish --workspace`, because the pinned cargo does not order
interdependent members on a *first* publish; see the workflow header.)
The ladder is resumable and rate-limit aware — crates.io throttles new
crates, so a 13-crate first publish runs in several passes. Members
marked `publish = false` (`aozora-corpus`, `aozora-conformance`,
`aozora-bench`, `aozora-trace`, `aozora-xtask`, plus the
`aozora-wasm` / `aozora-ffi` / `aozora-py` drivers that ship through
npm / GitHub Releases / PyPI) are skipped automatically.

Authentication is **OIDC Trusted Publishing** — no token in the repo.
`use_oidc` defaults to true and `rust-lang/crates-io-auth-action` mints a
30-minute, auto-revoked token. The one-time exception is the very first
publish of brand-new crates, which crates.io requires to go through an API
token; the [release secrets runbook](releasing-secrets.md) covers the
bootstrap-then-OIDC sequence and the per-crate trusted-publisher setup.

**Single front door, still.** The parser is built from many internal
crates (`aozora-spec`, `aozora-syntax`, `aozora-pipeline`,
`aozora-render`, `aozora-encoding`, `aozora-scan`, `aozora-veb`, plus
`aozora-cst` / `aozora-query` / `aozora-proptest`). They are now on
crates.io so the umbrella `aozora` crate can depend on them, but they
carry **no API-stability contract** — their crate descriptions say so,
and downstream consumers should depend on `aozora` alone.

### Why we publish before v1.0

Earlier this was deferred to v1.0 (every pre-1.0 minor may break the
API; a published name is load-bearing). We publish now because the
crate boundary has stabilised and claiming the `aozora*` namespace is
itself worth doing. The pre-1.0 SemVer contract above still holds — a
`0.x → 0.x+1` bump may break the API and is flagged by
`cargo-semver-checks`.

## Publishing to npm and PyPI

The browser (WASM) and Python drivers ship through their own manual
workflows, same `dry_run: true` default + gated `publish` job as crates:

```sh
gh workflow run publish-npm.yml  -f dry_run=false   # then approve
gh workflow run publish-pypi.yml -f dry_run=false   # then approve
```

Both authenticate via **OIDC Trusted Publishing** — no token secret in
steady state. `publish-npm.yml` builds with `wasm-pack build --target web
--release`, publishes `crates/aozora-wasm/pkg/`, and npm attaches a
provenance attestation automatically. `publish-pypi.yml` builds **one
`cp311-abi3` wheel per OS** (pyo3 `abi3-py311`, so a single wheel covers
CPython 3.11 → 3.14 and future 3.x — no per-Python-version matrix) plus
an sdist. Run `just smoke-py` first. Linux aarch64, macOS universal2, and
free-threaded (`3.13t`/`3.14t`, which abi3 cannot target) wheels are a
future cibuildwheel addition.

PyPI needs no bootstrap — a "pending publisher" covers the first upload —
while npm's first publish of a brand-new package needs a one-time token,
like crates.io. The full trusted-publisher + `release`-environment setup
for all three registries is in the
[release secrets runbook](releasing-secrets.md).

Cut these from the same `vX.Y.Z` tag as the GitHub Release so every
channel ships the same version. Run each workflow once with the
default `dry_run: true` first and confirm it's green before flipping
to `dry_run=false`.

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
