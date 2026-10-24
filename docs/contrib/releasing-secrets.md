# Release secrets and trusted publishing

Use this runbook to configure and verify release credentials in GitHub and package registries.

Keep long-lived publish tokens out of repository secrets.
Package registries use OIDC Trusted Publishing after any first-publication bootstrap.
Editor marketplaces need credentials only after a maintainer opts into the optional channel.

[ADR-0020](../adr/0020-release-secret-hardening-trusted-publishing.md) records the credential design.
[ADR-0038](../adr/0038-release-plz-owns-versioning-and-crates-io-publishing.md) covers crates.io and the two environments.

The [references](#references) identify each platform's official guidance.

## Security model

Both environments require owner approval and prohibit administrator bypass:

| | reviewer | branches / tags | why |
| --- | --- | --- | --- |
| `release` | `P4suta` | `main`, `v*`, `vscode-v*` | approve package publishers and draft assembly |
| `release-plz` | `P4suta` | `main` | approve release proposals, crates.io orchestration, and automation credential verification |

A job can't reach environment secrets or request its OIDC token before the protection rules pass.
GitHub waits for those rules before sending the job to a runner.
An environment-bound OIDC token carries `…:environment:<name>` in its `sub` claim, matching the registry's publisher rule.

Manual dispatches keep the following publication checks:

- Each channel publisher requires an exact `release-ready` commit and defaults `dry_run` to true.
  A dispatch downloads and verifies the commit's retained artifacts without rebuilding release bytes.
- `release-plz.yml` requires the exact qualified Release-PR merge commit.
  It checks the workspace version change, waits for successful `release-ready` evidence, and compares retained crate artifacts with a local reproduction before publishing.
  [ADR-0042](../adr/0042-release-ready-is-the-publish-authority.md) records the publication boundary.
- While `.github/RELEASE_FROZEN.md` exists, release-plz fails before either credential-bearing job starts.
  Turning off the workflow in GitHub Actions provides an independent server-side freeze.

## One-time setup

Create protected environments before the first gated run.
A workflow that names an absent environment creates it without protection rules.

### 1. The `release` environment

```sh
uid=$(gh api user --jq .id)
gh api -X PUT repos/P4suta/aozora/environments/release --input - <<EOF
{"wait_timer":0,"can_admins_bypass":false,
 "reviewers":[{"type":"User","id":$uid}],
 "deployment_branch_policy":{"protected_branches":false,"custom_branch_policies":true}}
EOF

gh api -X POST repos/P4suta/aozora/environments/release/deployment-branch-policies -f name='main'      -f type=branch
gh api -X POST repos/P4suta/aozora/environments/release/deployment-branch-policies -f name='v*'        -f type=tag
gh api -X POST repos/P4suta/aozora/environments/release/deployment-branch-policies -f name='vscode-v*' -f type=tag
```

### 2. The `release-plz` environment and GitHub App

```sh
uid=$(gh api user --jq .id)
gh api -X PUT repos/P4suta/aozora/environments/release-plz --input - <<EOF
{"wait_timer":0,"can_admins_bypass":false,
 "reviewers":[{"type":"User","id":$uid}],
 "deployment_branch_policy":{"protected_branches":false,"custom_branch_policies":true}}
EOF
gh api -X POST repos/P4suta/aozora/environments/release-plz/deployment-branch-policies -f name='main' -f type=branch
```

release-plz must push the `v*` tag as a GitHub App because tags pushed with `GITHUB_TOKEN` don't trigger downstream workflows.
Create a GitHub App with repository permissions **Contents: R/W** and **Pull requests: R/W**, without a webhook.
Install it on `P4suta/aozora`, generate a private key, and keep both the Client ID and numeric App ID.
The identifiers serve different operations.

The workflow fetches `GITHUB_APP_CLIENT_ID` and `GITHUB_APP_PRIVATE_KEY` from Doppler project `aozora`, configuration `automation`.
Repository variable `DOPPLER_AUTOMATION_IDENTITY_ID` selects the OIDC identity.
`verify-doppler.yml` checks read-only repository access and rejection of unrelated credentials through the protected `release-plz` environment.

The active `main` ruleset requires signed commits on the default branch and has no bypass actors.
Release PRs are squash-merged into GitHub web-flow-signed commits.
The `required` check gates merges.
`release-ready` qualifies the exact commit for publication.
The active `release tags` ruleset makes existing `v*` tags immutable and has no bypass actors.
See [Repository rulesets](../../.github/rulesets/README.md) to compare or re-apply the committed definitions.

An optional tag-creation restriction can limit `v*` creation to the release-plz App by its numeric **App ID**, rather than its Client ID.
Apply that separate rule only after watching release-plz cut a `v*` tag successfully.
Applying it earlier can leave a misconfigured App unable to create a tag.

### 3. Crate publication

Normal publication uses tokenless OIDC from the `release-plz` environment.
release-plz performs the exchange without `CARGO_REGISTRY_TOKEN` or `crates-io-auth-action`.

First publication requires a temporary token because of two crates.io constraints:

- **Trusted publishing can't create a crate.**
  The first publication of a new crate needs a token.
  [RFC 3691](https://rust-lang.github.io/rfcs/3691-trusted-publishing-cratesio.html) lists crate creation as a future possibility.
  crates.io has no equivalent to PyPI's pending publisher.
- **A publisher binds an exact workflow filename and environment.**
  The registry matches the token's `sub` claim to the configured publisher.

Bootstrap for the first release containing a new crate:

1. Before merging the Release PR, register Trusted Publishers for every existing crate.
   Use owner `P4suta`, repository `aozora`, workflow `release-plz.yml`, and environment `release-plz`.
2. Create a crates.io token with publish-new and publish-update scopes.
   Add it as the `CARGO_REGISTRY_TOKEN` secret on the `release-plz` environment.
3. In a bootstrap-only PR, add `CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}` to the `release-plz release` action's `env:`.
   Merge the bootstrap PR immediately before the Release PR.
4. Merge the Release PR.
   The token-authenticated run publishes new and existing public crates in dependency order and creates the release tag.
5. Register the newly created crate's Trusted Publisher.
   Remove the workflow reference in a cleanup PR, then delete the environment secret and revoke the token.

Don't publish new crates manually from the previous release tag.
Dependents can't compile with an unpublished dependency version, and `cargo publish` checks registry dependencies before uploading.

For crates with a publisher tied to the retired `publish-crates.yml`, delete and recreate the publisher entry with the current workflow.

Enable **Trusted Publishing only** per crate after a successful OIDC release.
The setting prevents token-based publication, so confirm the OIDC path before removing the fallback.
The token-authenticated bootstrap release supplies no OIDC evidence.
Use the next normal release to verify the exchange.

### 4. PyPI pending publisher

PyPI supports a pending publisher without a bootstrap token.
Add a pending publisher for distribution `aozora`, owner `P4suta`, repository `aozora`, workflow `publish-pypi.yml`, and environment `release`.

A pending publisher doesn't reserve the package name until first use.
Register and publish in the same release window.
The first OIDC upload creates the project and promotes the publisher.

### 5. Package trusted publisher

The `aozora-wasm` package already exists and needs no npm token bootstrap.
Register its trusted publisher with owner `P4suta`, repository `aozora`, workflow `publish-npm.yml`, and environment `release`.
Trusted Publishing requires npm CLI ≥ 11.5.1 and Node ≥ 22.14.0.
The workflow upgrades npm on the runner.

Turn off token publication in npm after the first successful OIDC release.

### 6. Optional editor marketplaces

Standard package releases omit editor marketplaces under [ADR-0049](../adr/0049-editor-marketplaces-are-opt-in-release-channels.md).
Create marketplace credentials only after explicitly opting into editor publication.
The channels require tokens in the protected `release` environment:

```sh
gh secret set VSCE_PAT --env release   # Azure DevOps PAT (Marketplace)
gh secret set OVSX_PAT --env release   # Open VSX token
```

### 7. Keep repository-level copies absent

Repository secrets lack the environment approval boundary.
Delete any old marketplace-token copies:

```sh
gh secret delete VSCE_PAT
gh secret delete OVSX_PAT
```

## Verification

```sh
gh secret list --env release          # empty is expected for package-only OIDC
gh secret list                        # publish tokens must be absent
gh secret list --env release-plz      # App credentials; bootstrap token only temporarily
gh api repos/P4suta/aozora/environments/release/deployment-branch-policies
```

The `Scorecard supply-chain security` workflow checks Token-Permissions and Pinned-Dependencies.

[Release process](release.md) describes publication.
Each protected publisher needs approval.
Publishing the completed GitHub draft requires a separate final action.

## References

- GitHub [security hardening](https://docs.github.com/en/actions/reference/security/secure-use)
- GitHub [environments](https://docs.github.com/en/actions/how-tos/deploy/configure-and-manage-deployments/manage-environments)
- GitHub [OIDC](https://docs.github.com/en/actions/concepts/security/openid-connect)
- [crates.io](https://crates.io/docs/trusted-publishing)
- [release-plz](https://release-plz.dev/docs/github/quickstart)
- [PyPI](https://docs.pypi.org/trusted-publishers/)
- [npm](https://docs.npmjs.com/trusted-publishers/)
- [OpenSSF Scorecard](https://github.com/ossf/scorecard)
