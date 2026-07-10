# Release process

aozora releases are **Release-PR driven** by
[release-plz](https://release-plz.dev/). Conventional Commits land on `main`;
release-plz keeps a single "Release PR" open that bumps the one
`[workspace.package].version` and the root `CHANGELOG.md`. Squash-merging that
PR makes release-plz publish every public crate to crates.io and cut a single
`vX.Y.Z` tag, which fires `.github/workflows/release.yml` (cross-platform
binaries + the GitHub Release) and the tag-driven PyPI / npm / Extism
publishers. **Humans never hand-edit a version or hand-push a release tag.**

> release-plz is **dormant until activated** — its workflow no-ops green until
> the GitHub App secrets exist. See [Activating release-plz](#activating-release-plz-one-time)
> for the one-time setup.

## Cutting a release

Pre-flight (run locally before landing the release-triggering changes, or
before approving the Release PR):

- [ ] `just fuzz-all-deep` is green — the 5-minute cargo-fuzz soak of all
  seven targets (pipeline `lex` / `classify` / `ffi_no_abort`, render
  `render_html` / `serialize_round_trip` / `catalogue_normalization`,
  encoding `decode_sjis`) reports zero crash / leak / oom artifacts.
- [ ] The `cross-os` workflow is green from a manual dispatch on the
  release commit (**Actions → cross-os → Run workflow**). It runs the
  workspace test suite natively on macOS and Windows — the platforms
  `release.yml` ships binaries for but only *builds* on — so a Windows-only
  path / CRLF regression cannot slip into a release. Local `just` cannot
  reproduce these runners (Docker-only policy's documented exception), so
  this green run is CI-authoritative.

```text
1. Land changes on main with Conventional Commits (feat / fix / perf / …).
   release-plz opens or updates the "Release PR" automatically.

2. Review the Release PR (it bumps [workspace.package].version 0.4.1 → 0.5.0
   and rewrites the [Unreleased] CHANGELOG section into the new version).

3. Add the `release: approved` label.
   (ci.yml's `release-gate` fails the PR's `ci-success` check without it;
   after labelling, re-run just the release-gate job to flip it green.)

4. SQUASH-merge the Release PR by hand.
   Squash, not rebase: GitHub web-flow GPG-signs the single merge commit, so
   main never receives release-plz's unsigned bot commits. (Auto-merge is
   force-disabled on release-plz PRs by no-automerge-on-release-pr.yml.)
```

That merge to `main` does the rest, unattended:

- `release-plz-release` publishes every public crate to crates.io in dependency
  order (OIDC trusted publishing — no token) and pushes the single `vX.Y.Z` tag.
- The tag fires `release.yml` (binaries + the GitHub Release) and the tag-driven
  `publish-pypi` / `publish-npm` / `publish-extism-wasm` workflows. These run in
  the reviewer-gated `release` environment, so approve the batch once under
  **Actions → the run → Review deployments**.

> **After the release**, bump the recommended git pin in
> `getting-started/install.md` to the new tag. It is the single source of truth
> for that pin (ADR-0009) and release-plz only rewrites `Cargo.toml` /
> `CHANGELOG.md`, never the docs — so this one literal is a manual follow-up.

### Who owns what

| Concern | Owner |
| --- | --- |
| Version bump, `CHANGELOG.md`, crates.io publish, the `v*` tag | **release-plz** (`release-plz.yml`) |
| Cross-platform binaries + the GitHub Release (notes from `CHANGELOG.md`) | `release.yml` |
| PyPI wheel / npm wasm / Extism `aozora.wasm` | the tag-driven `publish-*` workflows |
| VS Code extension (independent `vscode-v*` tag) | `release-vscode.yml` — **not** part of the release-plz flow |

## Sanity check after release

```sh
# Verify checksums (vX.Y.Z = the tag you just released)
curl -L -O https://github.com/P4suta/aozora/releases/download/vX.Y.Z/SHA256SUMS
curl -L -O https://github.com/P4suta/aozora/releases/download/vX.Y.Z/aozora-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz
sha256sum --check SHA256SUMS

# Verify the binary
tar -xzf aozora-vX.Y.Z-*.tar.gz
./aozora --version              # prints "aozora X.Y.Z"
```

## Build channels (dev / nightly / stable)

Every `aozora` / `aozora-lsp` binary reports a channel-aware version, so a
build's origin is unambiguous — a local build can never be mistaken for a
release. The string is resolved at build time by the `aozora-buildstamp` crate;
its format has a single source, the `xtask version` subcommand:

| Channel   | Example                            | Where it comes from                              |
| --------- | ---------------------------------- | ------------------------------------------------ |
| `dev`     | `0.5.0-dev+g3672e3f` (`.dirty`)    | any local `cargo build` (a workspace `.git`)     |
| `nightly` | `0.5.0-nightly.20260630+g3672e3f`  | `.github/workflows/nightly.yml` (tip of main)    |
| `stable`  | `0.5.0`                            | `.github/workflows/release.yml` / a crates.io install |

SemVer pre-release ordering gives `…-dev < …-nightly.<date> < X.Y.Z`, so a
non-release build always sorts below the release it heads toward. A crates.io
install reports the clean triple (no `.git`, no override → stable); only a
working-copy build carries `-dev`. The format comes from one place:

```sh
cargo run -p aozora-xtask -- version --channel nightly --date 20260630
cargo run -p aozora-xtask -- version --channel stable      # → 0.5.0
```

These channels are orthogonal to release-plz: `nightly.yml` / `release.yml`
*stamp* the build via `AOZORA_BUILD_VERSION`; release-plz *bumps* the workspace
version. They never fight over the same file.

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

## CHANGELOG generation

release-plz owns the single root `CHANGELOG.md`. Inside the Release PR it turns
the Conventional-Commits history since the last `v*` tag into a Keep-a-Changelog
section, grouped by commit type (its `[changelog]` config lives in
`release-plz.toml`, ported from the retired `cliff.toml`):

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
| `chore:` | (skipped when scope is `release`) |
| `revert:` | Reverted |

Non-conventional commits are silently skipped (they survive in `git log` but
don't pollute the changelog). There is **no** `just changelog` recipe — running
git-cliff by hand would fight release-plz over the file. To preview the next
changelog locally, run `release-plz update` (it writes `Cargo.toml` /
`CHANGELOG.md` in place; discard the spike with `git restore`). `release.yml`
reuses this same file: it extracts the new version's section from `CHANGELOG.md`
for the GitHub Release notes, so the changelog and the release page never drift.

## Why release-plz?

Earlier this repo cut releases by hand (`cargo set-version` + an annotated
`git tag`) and reviewed it as the right call for a single-author project. Two
things changed the calculus:

- **The workspace is now multi-crate and crates.io-published.** Eighteen public
  crates must bump and publish in lockstep, in dependency order. release-plz does
  exactly that natively from `version.workspace = true` — one
  `[workspace.package].version` bump cascades to every crate and it publishes
  them topologically. The previous hand-maintained ladder in `publish-crates.yml`
  is gone.
- **A Release PR is a better gate than a local tag.** The bump + CHANGELOG land
  as a reviewable PR; the deliberate human act is a labelled squash-merge, not a
  local `git push --tags`. `cargo-semver-checks` runs on that PR (`semver_check`
  in `release-plz.toml`).

`cargo-dist` was not adopted (ADR-0021): it is opinionated about archive layout
(`bin/` + `share/`), while aozora's archive is flat (`aozora` + `LICENSE-*` +
`NOTICE` + `README.md`), and it has no first-class clap-completion / mangen /
FFI-cdylib generation — so the hand-written `release.yml` stays for binaries.
release-plz owns versioning + crates.io; `release.yml` owns the binaries and
GitHub Release. This deliberately diverges from the rest of the ecosystem, which
uses cargo-dist for its simpler single-binary releases.

## Why three release targets and not five?

`release.yml` builds:

- `x86_64-unknown-linux-gnu` (linux x86_64)
- `aarch64-apple-darwin` (macOS arm64)
- `x86_64-pc-windows-msvc` (windows x86_64)

We *don't* build `x86_64-apple-darwin` (macOS Intel — Apple deprecated the
platform; arm64 covers all current Apple Silicon machines) or
`aarch64-unknown-linux-gnu` (linux arm64 — covered by `cargo install` from
source for the niche ARM Linux deployment case).

Adding a target is one line in `release.yml`; we add them when a real consumer
asks for a binary build of one. Pre-emptive coverage isn't worth the CI minutes.

## Pre-1.0 SemVer

aozora is currently in the `0.x` series. The contract:

- `0.x.y` → `0.x.y+1`: patches and additions, no breaks. Always safe to upgrade.
- `0.x.y` → `0.x+1.0`: may break the API. `cargo-semver-checks` flags the breaks
  on the Release PR; the changelog records them.
- `0.x.y` → `1.0.0`: the API freeze. Post-1.0, breaking changes collect on a
  `next` branch and ship in a major bump.

The MSRV pin (`rust-toolchain.toml`) advances on its own cadence, roughly
quarterly. MSRV bumps are *not* breaking under our pre-1.0 contract — consumers
that need a frozen MSRV pin a release tag.

When you raise the MSRV, bump the **Dockerfile `FROM rust:` base in the same
commit** so the dev image keeps building on exactly the pinned channel (one
toolchain, no dead second one). Dependabot deliberately ignores the `rust` base
image (`.github/dependabot.yml`) precisely so it cannot drift ahead of
`rust-toolchain.toml`, so this base bump is manual. Resolve the new digest with
`docker buildx imagetools inspect rust:<ver>-bookworm`.

## Publishing to crates.io

Live since the first crates.io publish; owned by **release-plz**. When the Release PR merges, the
`release-plz-release` job publishes every publishable crate at the new version
in dependency order, tokenless via **crates.io OIDC trusted publishing** (no
`CARGO_REGISTRY_TOKEN` — release-plz performs the OIDC exchange itself). Members
marked `publish = false` (`aozora-corpus`, `aozora-conformance`, `aozora-bench`,
`aozora-trace`, `aozora-xtask`, plus the `aozora-wasm` / `aozora-ffi` /
`aozora-py` / `aozora-extism` drivers that ship through npm / GitHub Releases /
PyPI) are skipped automatically.

**Single front door, still.** The parser is built from many internal crates
(`aozora-spec`, `aozora-syntax`, `aozora-pipeline`, `aozora-render`,
`aozora-encoding`, `aozora-scan`, `aozora-veb`, `aozora-cst`, `aozora-query`,
`aozora-proptest`, `aozora-fmt`, `tree-sitter-aozora`).
They are on crates.io so the umbrella `aozora` crate and the `aozora-lsp` /
`aozora-cli` binaries can depend on them, but they carry **no API-stability
contract** — their crate descriptions say so, and downstream consumers should
depend on `aozora` alone.

The per-crate trusted-publisher setup (and the one-time first-publish bootstrap
for brand-new crates, which crates.io requires to go through a token) is in the
[release secrets runbook](releasing-secrets.md).

## Publishing to npm and PyPI

The browser (WASM) and Python drivers fan out **automatically from the same
`v*` tag** release-plz cuts; `workflow_dispatch` stays as a manual fallback
(dry-run by default). Both authenticate via **OIDC Trusted Publishing** — no
token secret in steady state, and both publish jobs stay in the reviewer-gated
`release` environment (approve once under Review deployments).

- `publish-npm.yml` builds with `wasm-pack build --target web --release`,
  publishes `crates/aozora-wasm/pkg/`, and npm attaches a provenance attestation
  automatically. A tag push always takes the OIDC path.
- `publish-pypi.yml` builds **one `cp311-abi3` wheel per OS** (pyo3 `abi3-py311`,
  so a single wheel covers CPython 3.11 → 3.14 and future 3.x — no
  per-Python-version matrix) plus an sdist. Run `just smoke-py` before a release.
- `publish-extism-wasm.yml` attaches the portable `aozora.wasm` to the GitHub
  Release that `release.yml` cuts from the same tag; it waits for that Release to
  exist before uploading.

PyPI needs no bootstrap — a "pending publisher" covers the first upload — while
npm's first publish of a brand-new package needs a one-time token, like
crates.io. The full trusted-publisher + `release`-environment setup for all
registries is in the [release secrets runbook](releasing-secrets.md).

## Activating release-plz (one-time)

release-plz ships **dormant**: `release-plz.yml` runs green and no-ops until the
GitHub App secrets exist. Bringing it live is a credential/registry/ruleset
exercise — all of it manual, in this order. The two ruleset commands live in
`.github/rulesets/README.md`; the crates.io / App secret details are in the
[release secrets runbook](releasing-secrets.md).

1. **Create the GitHub App** (org or personal): repository permissions
   **Contents: R/W** + **Pull requests: R/W**, no webhook. Install it on
   `P4suta/aozora`, generate a private key, and record both the **Client ID**
   (for the token) and the numeric **App ID** (for the ruleset bypass actor).
2. **Set the `release-plz` environment secrets** `RELEASE_PLZ_APP_CLIENT_ID` and
   `RELEASE_PLZ_APP_PRIVATE_KEY` (create the environment first — step 3).
   `release-plz.yml` reads them via its `HAS_APP` gate; once both exist, trigger
   the first run with `gh workflow run release-plz.yml` (or the next push to `main`).
3. **Create the `release-plz` environment** — deployment-branch policy `main`
   only, **no required reviewers**, no wait timer (the publish must run
   unattended; the Release-PR merge is the human gate).
4. **Apply the signature bypass** on `require-signed-commits` (the App pushes the
   bump commit unsigned to its `release-plz-*` branch). See the rulesets README.
5. **Bootstrap the new crates** on crates.io (trusted publishing can't do a
   first publish) and **register the trusted publishers** for all 18 crates
   against `release-plz.yml` / the `release-plz` environment — see the
   [release secrets runbook](releasing-secrets.md).
6. **Trigger release-plz** (`gh workflow run release-plz.yml`) to open the first
   Release PR, add `release: approved`, and squash-merge it. Verify the chain:
   crates.io publishes, the `vX.Y.Z` tag is pushed, `release.yml` + the downstream
   publishers fan out.
7. **Apply the `v*` tag-creation lock last** (App-only tag creation; see the
   rulesets README) — doing it earlier would block the current manual tag flow.

Until step 2 the whole pipeline is inert, so the scaffolding can land long before
the App is created.

## Code signing

Release binaries are **not** CA code-signed (no Authenticode on the Windows
`.exe`, no Apple Developer ID / notarization on the macOS build). This is a
deliberate pre-1.0 decision.

What we ship instead — and why it covers the current audience:

- **Build provenance attestation** (`actions/attest-build-provenance`, since
  the first tagged release): every archive carries a Sigstore-backed SLSA provenance statement,
  verifiable with `gh attestation verify <archive> --repo P4suta/aozora` — no
  certificates, no CA. It proves *which CI built which artefact from which
  source*: a supply-chain control, **not** an OS-level execution-trust signal.
- **SHA256SUMS** for integrity; **signed git tags / commits** for authorship.

CA code signing solves a *different* problem — suppressing the Windows
SmartScreen / macOS Gatekeeper "unknown publisher" prompt for end users who
double-click a downloaded binary. For a parser library + developer CLI installed
via `cargo install` / package managers, that prompt is low-friction, so the
recurring cost and operational overhead (HSM-stored keys mandatory since
2023-06; ≤458-day cert validity since 2026-03) is not justified yet.

When we revisit this (post-1.0, if desktop double-click installs become a real
distribution path):

- **Windows** → [SignPath Foundation](https://signpath.io/solutions/open-source-community)
  free OSS code signing (Sectigo-issued, HSM-backed, CI-integrated). Note the
  2024 SmartScreen change: EV no longer buys *instant* trust — both OV and EV
  build reputation organically over downloads.
- **macOS** → Apple Developer ID ($99/yr Apple Developer Program) +
  notarization. Third-party CA certs (e.g. ssl.com) do **not** satisfy
  Gatekeeper; only an Apple-issued Developer ID does.
- A paid CA (ssl.com eSigner, etc.) was evaluated and rejected: it covers
  Windows only, no longer removes the first-run warning on day one, and adds a
  yearly cost the project does not need pre-1.0.

## See also

- [Development loop](dev.md) — the local pre-flight commands.
- [Testing strategy](testing.md) — `prop-deep` and corpus sweep details.
- [Release secrets & Trusted Publishing](releasing-secrets.md) — the App,
  environments, and per-registry trusted-publisher setup.
