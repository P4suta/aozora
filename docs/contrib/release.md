# Release process

[Real-work release gate](real-work-release-gate.md) describes the all-edition, seven-distribution checks and first-baseline procedure.

Releases are Release-PR driven by [release-plz](https://release-plz.dev/).
Humans never edit a version or push a release tag.

## Publication freeze

When `.github/RELEASE_FROZEN.md` exists, release-plz fails before either credential-bearing job starts.
Also turn off the workflow in GitHub Actions.
The repository latch and server-side setting provide independent controls, so an accidental enable doesn't resume publication.

Rearming requires a release decision.
Remove the latch in a dedicated reviewed PR while the workflow remains off.
Verify the merge created no tag or publication.
Enable the workflow only after a maintainer explicitly starts a new release checkpoint.

Before enabling the workflow, run `just rearm-preflight`, which invokes `xtask release preflight`.
Preflight checks environment credentials and protection, the server-side tag ruleset, successful `release-ready` evidence for the exact commit, and existing registry artifacts.
It refuses missing prerequisites and identifies remaining manual checks.
Then run `just rearm-rehearse`, which invokes `xtask release rehearse`.
Rehearsal dispatches the PyPI and npm publishers with `dry_run` so their `qualify` jobs execute before tag creation.
The `drift-gate` also runs `xtask release check` to compare committed rulesets and native software bill of materials paths with the release workflow.

## Verification gates

Every pull request and merge group runs the fast gate that `required` summarizes.
`release-ready` runs the heavy checks, from coverage and the corpus sweep to mutation, fuzzing, sanitizers, and other platforms.
The [gate split](../adr/fast-pull-request-gate-and-release-gate.md) records where each check runs.

## Cutting a release

1. Merge Conventional Commits into `main`.
   release-plz opens or updates the Release PR with the workspace version and `CHANGELOG.md`.
2. Review that diff and the `release-ready` result.
   The result covers the fixed external specification and corpus, tests, documentation, property tests, fuzzing, sanitizers, and mutation testing.
   It also covers native shipping platforms, performance contracts, generated wire surfaces, and installation from each distribution artifact.
3. Add `release: approved` after review.
   Branch protection requires the `required` check.
   `release-ready` qualifies the exact commit for publication.
   The label grants no bypass of either gate.
4. Squash-merge the Release PR manually.
   GitHub web-flow signs the resulting commit, and Release PRs require a manual merge.
5. Wait for release-plz to publish the public crates in dependency order and create the `vX.Y.Z` tag.
6. First approve `release.yml` in the protected `release` environment.
   It creates the draft GitHub Release and attaches the native and Go artifacts.
7. Approve `publish-extism-wasm.yml`, `publish-npm.yml`, and `publish-pypi.yml` as separate deployments.
   Don't dispatch or approve `release-vscode.yml` as part of a package release.
8. Verify every intended registry version, draft asset, checksum, and attestation.
   Publish the draft in GitHub only after they all match.

If release-plz fails before creating the tag, fix the workflow and dispatch it with the exact qualified Release PR merge commit:

```sh
commit=<40-character-release-merge-commit>
gh workflow run release-plz.yml -f commit="$commit"
```

The recovery checkout, successful `release-ready` run, artifact manifest, and published crate bytes must all resolve to the supplied commit.

The Release-PR merge adds the version-changing commit to `main`, triggering `release-ready`.
After success, `release-fan-in` dispatches release-plz and the Pages documentation deployment for the exact commit.
release-plz checks the successful gate result again before publishing and creating the tag.
Tag-driven jobs publish the verified package artifacts.
`release-ready` also qualifies editor extension artifacts, but editor marketplaces require an explicit opt-in under [ADR-0049](../adr/0049-editor-marketplaces-are-opt-in-release-channels.md).
Manual recovery with `gh workflow run release-plz.yml -f commit=…` uses the same dispatch entry point.

Only draft GitHub Releases remain mutable.
Retry a failed native or Extism upload before publishing the draft.
Publication is the final irreversible action under [ADR-0050](../adr/0050-immutable-releases-are-assembled-as-drafts.md).

For a retry, resolve the tag commit once and pass it to the affected channel:

```sh
tag=vX.Y.Z
commit=$(git rev-parse "${tag}^{commit}")
gh workflow run release.yml -f commit="$commit" -f tag="$tag" -f dry_run=false
gh workflow run publish-npm.yml -f commit="$commit" -f dry_run=false
gh workflow run publish-pypi.yml -f commit="$commit" -f dry_run=false
gh workflow run publish-extism-wasm.yml -f commit="$commit" -f tag="$tag" -f dry_run=false
```

After downloading an asset, verify the immutable-release attestation and build provenance:

```sh
gh release verify vX.Y.Z --repo P4suta/aozora
gh release verify-asset vX.Y.Z ./aozora-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz --repo P4suta/aozora
gh attestation verify aozora-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz --repo P4suta/aozora
```

## Semantic versioning

- `0.x.y` to `0.x.y+1` contains no intentional breaking change.
- `0.x.y` to `0.x+1.0` may break compatibility, and release checks compare the change with the published API.
- `1.0.0` freezes the redesigned public façade.

An increase to the supported Rust version floor follows the six-month contract in [ADR-0034](../adr/0034-separate-toolchain-channel-from-msrv.md).

## See also

- [ADR-0042](../adr/0042-release-ready-is-the-publish-authority.md)
- [ADR-0051](../adr/0051-ci-and-release-ready-split-event-driven-release-fan-in-and-the-actions-concurrency-budget.md)
- [ADR-0052](../adr/0052-code-gate-reuse-by-code-identity.md)
- [ADR-0049](../adr/0049-editor-marketplaces-are-opt-in-release-channels.md)
- [ADR-0050](../adr/0050-immutable-releases-are-assembled-as-drafts.md)
- [Release secrets and Trusted Publishing](releasing-secrets.md)
- [ADR-0037](../adr/0037-release-binaries-are-not-ca-code-signed.md)
