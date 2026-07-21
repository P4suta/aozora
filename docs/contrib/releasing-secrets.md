# Release secrets & Trusted Publishing

The operational runbook for the release credentials. Almost none of this
is visible in the tree: it is GitHub-server-side and registry-side state,
so this page is its only record.

The goal: **no long-lived publish token sits in the repository.** Package
registries use OIDC Trusted Publishing after any first-publish bootstrap.
Editor marketplaces are not part of the standard package release and need no
credentials until a maintainer explicitly opts into that channel.

Why it is shaped this way is in
[ADR-0020](../adr/0020-release-secret-hardening-trusted-publishing.md)
and [ADR-0038](../adr/0038-release-plz-owns-versioning-and-crates-io-publishing.md)
— the latter covers crates.io and the two environments. This page is the
how.

Everything below follows each platform's official guidance; the
[references](#references) are the primary sources.

## Security model

Two environments, deliberately different:

| | reviewer | branches / tags | why |
| --- | --- | --- | --- |
| `release` | required | `main`, `v*` | you approve each package publisher and draft assembly |
| `release-plz` | **none** | `main` | the gate is the Release-PR merge; a second approval would only stall an unattended publish |

A job cannot reach environment secrets **or** the OIDC token until the
protection rules pass, because GitHub does not send it to a runner before
then. An OIDC token minted under an environment carries
`…:environment:<name>` in its `sub` claim, which is what the registry's
publisher rule matches.

What stops a stray **Run workflow** click depends on the publisher:

- Every channel publisher requires an exact `release-ready` commit on manual
  dispatch and defaults `dry_run` to true. A dispatch downloads and verifies
  that commit's retained artifacts; it never rebuilds release bytes.
- `release-plz.yml` requires the exact qualified Release-PR merge commit on
  manual dispatch. It checks that commit changes the workspace version, waits
  for its successful `release-ready` run, and compares the retained crate
  artifacts with a local reproduction before publishing. See
  [ADR-0042](../adr/0042-release-ready-is-the-publish-authority.md).
- While `.github/RELEASE_FROZEN.md` exists, release-plz fails before either
  credential-bearing job starts. Manually disabling the workflow in GitHub
  Actions provides an independent server-side freeze.

## One-time setup

> A workflow naming an environment that does not exist yet **auto-creates
> it without protection rules** on the first run. Create the environments
> before the first gated run, not after.

### 1. The `release` environment

```sh
uid=$(gh api user --jq .id)
gh api -X PUT repos/P4suta/aozora/environments/release --input - <<EOF
{"wait_timer":0,
 "reviewers":[{"type":"User","id":$uid}],
 "deployment_branch_policy":{"protected_branches":false,"custom_branch_policies":true}}
EOF

gh api -X POST repos/P4suta/aozora/environments/release/deployment-branch-policies -f name='main'      -f type=branch
gh api -X POST repos/P4suta/aozora/environments/release/deployment-branch-policies -f name='v*'        -f type=tag
```

### 2. The `release-plz` environment + GitHub App

```sh
gh api -X PUT repos/P4suta/aozora/environments/release-plz --input - <<EOF
{"wait_timer":0,
 "deployment_branch_policy":{"protected_branches":false,"custom_branch_policies":true}}
EOF
gh api -X POST repos/P4suta/aozora/environments/release-plz/deployment-branch-policies -f name='main' -f type=branch
```

release-plz must push the `v*` tag as a **GitHub App** — a tag pushed by
the default `GITHUB_TOKEN` does not trigger the downstream workflows.
Create an App (org or personal) with repository permissions **Contents:
R/W** + **Pull requests: R/W** and no webhook, install it on
`P4suta/aozora`, generate a private key, and note **both** the Client ID
and the numeric App ID — they are used in different places.

```sh
gh secret set RELEASE_PLZ_APP_CLIENT_ID --env release-plz   # the Client ID (Iv23…)
gh secret set RELEASE_PLZ_APP_PRIVATE_KEY --env release-plz # the .pem contents
```

The two ruleset changes — signature bypass on `release-plz-*`, and the
`v*` tag-creation lock — key on the numeric **App ID**, not the Client ID.
Apply them per `.github/rulesets/README.md`. **Apply the tag lock last:** it
restricts `v*` creation to the App, so apply it only once you have watched
release-plz cut a `v*` tag. Lock first and a misconfigured App leaves the tag
uncuttable by anyone.

### 3. crates.io

Steady state is tokenless OIDC from the `release-plz` environment.
release-plz performs the OIDC exchange itself, so there is no
`CARGO_REGISTRY_TOKEN` and `crates-io-auth-action` is not used.

Getting there costs one token, once, because of two crates.io facts:

- **Trusted publishing cannot create a crate.** The first publish of a new
  crate needs a token.
  [RFC 3691](https://rust-lang.github.io/rfcs/3691-trusted-publishing-cratesio.html)
  lists crate creation only as a future possibility; PyPI's pending
  publisher has no crates.io equivalent.
- **A publisher is bound to an exact workflow filename + environment**,
  because that is what the `sub` claim is matched against.

Bootstrap for the first release containing a new crate:

1. Before merging the Release PR, register Trusted Publishers for every crate
   that already exists. Use owner `P4suta`, repository `aozora`, workflow
   `release-plz.yml`, and environment `release-plz`.
2. Create a crates.io token with publish-new and publish-update scopes. Add it
   as the `CARGO_REGISTRY_TOKEN` secret on the `release-plz` environment.
3. In a small bootstrap-only PR, add
   `CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}` only to the
   `release-plz release` action's `env:`. Merge it immediately before the
   Release PR.
4. Merge the Release PR. This one token-authenticated run publishes both new
   and existing public crates in dependency order and creates the release tag.
5. Register the newly created crate's Trusted Publisher. In a cleanup PR,
   remove the workflow reference, then delete the environment secret and
   revoke the token.

Do **not** hand-pre-create the new crates against the previous release
tag. They did not exist at that tag, so dependents cannot compile against
the umbrella's published version — `cargo publish` verifies against the
registry and fails before uploading.

Crates carried over from the retired `publish-crates.yml` still point at the
old workflow, and crates.io has no edit UI. Delete and re-add those publisher
entries.

Then enable **Trusted Publishing only** per crate, so a leaked token
cannot publish aozora. Do this *after* a green OIDC release, never before:
a failed OIDC publish would otherwise leave no way back in. The bootstrap
release does not prove OIDC because the token is present; use the next normal
release as that proof.

### 4. PyPI — tokenless from day one

PyPI has a **pending publisher**, so no bootstrap. Add one (Publishing →
pending publisher) with the distribution name of `aozora`, owner
`P4suta`, repository `aozora`, workflow `publish-pypi.yml`, environment
`release`.

Then publish promptly — a pending publisher does **not** reserve the name
until first use, so register and publish back to back. The first OIDC
upload creates the project and promotes the publisher.

### 5. npm — existing package, configure OIDC directly

The `aozora-wasm` package already exists, so no npm token bootstrap is needed.
Register its trusted publisher with owner `P4suta`, repository `aozora`,
workflow `publish-npm.yml`, and environment `release`. Trusted Publishing
requires npm CLI ≥ 11.5.1 and Node ≥ 22.14.0; the workflow upgrades npm on the
runner.

Enable npm's setting that disallows token publication after the first green
OIDC release.

### 6. VS Code Marketplace & Open VSX — optional, no OIDC

These are disabled for package releases under
[ADR-0049](../adr/0049-editor-marketplaces-are-opt-in-release-channels.md).
Do not create their credentials until explicitly opting into editor
publication. No OIDC publishing exists, so any future credentials belong on
the `release` environment behind its approval gate:

```sh
gh secret set VSCE_PAT --env release   # Azure DevOps PAT (Marketplace)
gh secret set OVSX_PAT --env release   # Open VSX token
```

### 7. Keep repository-level copies absent

Repository secrets are readable by any workflow run, so they silently
defeat the environment gate. If any old copies exist:

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

The `Scorecard supply-chain security` workflow keeps the Token-Permissions
and Pinned-Dependencies posture from regressing.

The release itself is in [Release process](release.md). Each protected
publisher needs approval, and publishing the completed GitHub draft is a
separate final action.

## References

- GitHub — [security hardening](https://docs.github.com/en/actions/reference/security/secure-use),
  [environments](https://docs.github.com/en/actions/how-tos/deploy/configure-and-manage-deployments/manage-environments),
  [OIDC](https://docs.github.com/en/actions/concepts/security/openid-connect)
- [crates.io](https://crates.io/docs/trusted-publishing) ·
  [release-plz](https://release-plz.dev/docs/github/quickstart) ·
  [PyPI](https://docs.pypi.org/trusted-publishers/) ·
  [npm](https://docs.npmjs.com/trusted-publishers/)
- [OpenSSF Scorecard](https://github.com/ossf/scorecard)
