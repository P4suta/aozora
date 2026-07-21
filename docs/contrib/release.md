# Release process

Releases are Release-PR driven by [release-plz](https://release-plz.dev/).
Humans never edit a version or push a release tag.

## Publication freeze

When `.github/RELEASE_FROZEN.md` exists, release-plz fails before either
credential-bearing job can start. The GitHub Actions workflow should also be
manually disabled. The repository latch and server-side disable are independent
so an accidental enable does not resume publication.

Rearming is a release decision, not ordinary maintenance. Remove the latch in
a dedicated reviewed PR while the workflow remains disabled, verify the merge
did not create a tag or publication, and enable the workflow only after a
maintainer explicitly starts a new release checkpoint.

Before enabling, run `just rearm-preflight` (`xtask release preflight`). It
verifies the deployed preconditions a green CI never proves — the `release-plz`
and `release` environment secrets and protection, the server-side tag ruleset,
a completed-success `release-ready` for the exact commit, and the first-publish
registry residue — and fails closed on any gap, printing the irreducibly-manual
items to acknowledge. `just rearm-rehearse` (`xtask release rehearse`) then
fires the PyPI and npm publishers' `dry_run` dispatches so their `qualify` jobs
run before the irreversible tag. The offline half is already a `drift-gate`
check (`xtask release check`): the tag and main rulesets still encode their
rules, and the PR-time native-SBOM path set still mirrors `release.yml`'s
tag-time expectation.

## Cutting a release

1. Land Conventional Commits on `main`. release-plz opens or updates the
   Release PR with the workspace version and `CHANGELOG.md`.
2. Review that diff and the `release-ready` result. The result covers the fixed
   external specification and corpus, all tests and documentation, deep
   property/fuzz/sanitizer/mutation work, native shipping platforms,
   performance contracts, generated wire surfaces, and installation from every
   distribution artifact.
3. Add `release: approved` after review. Branch protection still requires
   `release-ready`; the label cannot make a failed or absent proof mergeable.
4. Squash-merge the Release PR by hand. GitHub web-flow signs the resulting
   commit; auto-merge remains disabled for Release PRs.
5. Wait for release-plz to publish the public crates in dependency order and
   create the `vX.Y.Z` tag.
6. Approve `release.yml` in the protected `release` environment first. It
   creates the draft GitHub Release and attaches the native and Go artifacts.
7. Approve `publish-extism-wasm.yml`, `publish-npm.yml`, and
   `publish-pypi.yml`. Each is a separate deployment. Do not dispatch or
   approve `release-vscode.yml` as part of a package release.
8. Verify every intended registry version, draft asset, checksum, and
   attestation. Publish the draft in the GitHub UI only after they all match.

If release-plz fails before creating the tag, fix the workflow at its source,
then dispatch it with the exact qualified Release PR merge commit:

```sh
commit=<40-character-release-merge-commit>
gh workflow run release-plz.yml -f commit="$commit"
```

The recovery checkout, successful `release-ready` run, artifact manifest, and
published crate bytes must all resolve to that commit.

The Release-PR merge lands the version-changing commit on `main`, which runs
`release-ready`. On success, `release-fan-in` dispatches release-plz (and the
Pages docs deploy) for that exact commit rather than either workflow polling
for the result; release-plz re-checks that `release-ready` is
completed-success for the commit, then publishes and creates the tag. The
tag-driven jobs publish only the already-verified package artifacts. VSIX
artifacts remain qualified by `release-ready`, but editor marketplaces are
opt-in under
[ADR-0049](../adr/0049-editor-marketplaces-are-opt-in-release-channels.md).
The manual `gh workflow run release-plz.yml -f commit=…` recovery above is the
same dispatch entrypoint, driven by hand instead of by `release-fan-in`.

The GitHub Release remains mutable only while it is a draft. Retry a failed
native or Extism upload before publishing it. Publishing the draft is the final
irreversible action under
[ADR-0050](../adr/0050-immutable-releases-are-assembled-as-drafts.md).

For a retry, resolve the tag commit once and pass it to the affected channel:

```sh
tag=vX.Y.Z
commit=$(git rev-parse "${tag}^{commit}")
gh workflow run release.yml -f commit="$commit" -f tag="$tag" -f dry_run=false
gh workflow run publish-npm.yml -f commit="$commit" -f dry_run=false
gh workflow run publish-pypi.yml -f commit="$commit" -f dry_run=false
gh workflow run publish-extism-wasm.yml -f commit="$commit" -f tag="$tag" -f dry_run=false
```

After downloading an asset, verify both the immutable-release attestation and
the build provenance:

```sh
gh release verify vX.Y.Z --repo P4suta/aozora
gh release verify-asset vX.Y.Z ./aozora-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz --repo P4suta/aozora
gh attestation verify aozora-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz --repo P4suta/aozora
```

## SemVer

- `0.x.y` to `0.x.y+1` contains no intentional breaking change.
- `0.x.y` to `0.x+1.0` may break and is checked against the published API.
- `1.0.0` freezes the redesigned public façade.

An MSRV bump follows the six-month contract in
[ADR-0034](../adr/0034-separate-toolchain-channel-from-msrv.md).

## See also

- [ADR-0042](../adr/0042-release-ready-is-the-publish-authority.md)
- [ADR-0051](../adr/0051-ci-and-release-ready-split-event-driven-release-fan-in-and-the-actions-concurrency-budget.md)
- [ADR-0052](../adr/0052-code-gate-reuse-by-code-identity.md)
- [ADR-0049](../adr/0049-editor-marketplaces-are-opt-in-release-channels.md)
- [ADR-0050](../adr/0050-immutable-releases-are-assembled-as-drafts.md)
- [Release secrets and Trusted Publishing](releasing-secrets.md)
- [ADR-0037](../adr/0037-release-binaries-are-not-ca-code-signed.md)
