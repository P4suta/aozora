# Release secrets & Trusted Publishing

The operational runbook for the release credentials. Almost none of this
is visible in the tree: it is GitHub-server-side and registry-side state,
so this page is its only record.

The goal: **no long-lived publish token sits in the repository.** Where a
registry supports OIDC Trusted Publishing we mint a short-lived token at
publish time. Where it does not (VS Code Marketplace, Open VSX) the token
will live as an *Environment* secret behind an approval gate, never as a
repository secret.

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
| `release` | required | `main`, `v*`, `vscode-v*` | you approve each publish batch |
| `release-plz` | **none** | `main` | the gate is the Release-PR merge; a second approval would only stall an unattended publish |

A job cannot reach environment secrets **or** the OIDC token until the
protection rules pass, because GitHub does not send it to a runner before
then. An OIDC token minted under an environment carries
`…:environment:<name>` in its `sub` claim, which is what the registry's
publisher rule matches.

What stops a stray **Run workflow** click is not one mechanism but three,
and it is worth knowing which one you are relying on:

- `release.yml` — an event guard: the publish job requires a `refs/tags/`
  push, so a dispatch builds and stops.
- `publish-pypi` / `publish-npm` / `publish-extism-wasm` — `dry_run: true`
  is the input default.
- `release-plz.yml` — **neither**, and the button is live. Its
  `workflow_dispatch` takes no inputs and `release-plz-release` has no
  event guard, so a dispatch on `main` is equivalent to a push on `main`.
  What bounds both is not in the workflow at all: `release-plz.toml`'s
  `release_always = false` releases only from a Release-PR merge commit,
  so a dispatch on an ordinary `main` commit does nothing, and one on the
  merge commit re-runs a publish that is idempotent against crates.io —
  which is how you finish a release that failed. See
  [ADR-0039](../adr/0039-release-plzs-manual-trigger-stays-unguarded.md).

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
gh api -X POST repos/P4suta/aozora/environments/release/deployment-branch-policies -f name='vscode-v*' -f type=tag
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

Bootstrap:

1. Add `CARGO_REGISTRY_TOKEN` (scopes: publish-new **and** publish-update)
   as a `release-plz` environment secret and reference it from
   `release-plz-release`'s `env:`.
2. Merge the Release PR. Authenticated by token, this is the one run that
   can bring new crates into existence.
3. Remove the `env:` line, delete the secret, revoke the token.

Do **not** hand-pre-create the new crates against the previous release
tag. They did not exist at that tag, so dependents cannot compile against
the umbrella's published version — `cargo publish` verifies against the
registry and fails before uploading.

Register a publisher per publishable crate (crate → Settings → Trusted
Publishing → Add → GitHub): owner `P4suta`, repository `aozora`, workflow
**`release-plz.yml`**, environment **`release-plz`**. Crates carried over
from the retired `publish-crates.yml` still point at it, and crates.io has
no edit UI — **delete and re-add** those.

Then enable **Trusted Publishing only** per crate, so a leaked token
cannot publish aozora. Do this *after* a green OIDC release, never before:
a failed OIDC publish would otherwise leave no way back in.

### 4. PyPI — tokenless from day one

PyPI has a **pending publisher**, so no bootstrap. Add one (Publishing →
pending publisher) with the distribution name of `aozora-py`, owner
`P4suta`, repository `aozora`, workflow `publish-pypi.yml`, environment
`release`.

Then publish promptly — a pending publisher does **not** reserve the name
until first use, so register and publish back to back. The first OIDC
upload creates the project and promotes the publisher.

### 5. npm — bootstrap required

Unlike PyPI, npm requires the package to **already exist** before a
trusted publisher can be configured, and the account needs 2FA. So npm
needs a one-time token, like crates.io. (Trusted Publishing also needs npm
CLI ≥ 11.5.1 and Node ≥ 22.14.0; the workflow upgrades npm on the runner.)

1. Create a granular-access token that can publish `aozora-wasm`, add it
   as the `NPM_TOKEN` secret on the `release` environment.
2. `gh workflow run publish-npm.yml -f dry_run=false -f use_oidc=false`
3. Register the trusted publisher: package → Settings → Trusted Publisher
   → GitHub Actions, workflow `publish-npm.yml`, environment `release`.
4. Delete the `NPM_TOKEN` secret. From then on a `v*` tag publishes
   automatically and npm attaches provenance itself.

### 6. VS Code Marketplace & Open VSX — no OIDC

No OIDC publishing exists, so these tokens persist. Once created they will
live on the environment, behind the approval gate:

```sh
gh secret set VSCE_PAT --env release   # Azure DevOps PAT (Marketplace)
gh secret set OVSX_PAT --env release   # Open VSX token (optional)
```

### 7. Delete the repository-level copies

Repository secrets are readable by any workflow run, so they silently
defeat the environment gate. Once the values live on `release`:

```sh
gh secret delete VSCE_PAT
gh secret delete OVSX_PAT
gh secret delete NPM_TOKEN
```

## Verification

```sh
gh secret list --env release          # holds the non-OIDC tokens
gh secret list                        # VSCE_PAT / OVSX_PAT must be ABSENT
gh api repos/P4suta/aozora/environments/release/deployment-branch-policies
```

The `Scorecard supply-chain security` workflow keeps the Token-Permissions
and Pinned-Dependencies posture from regressing.

The release itself is in [Release process](release.md) — from there, the
only manual step is approving the publish batch once.

## References

- GitHub — [security hardening](https://docs.github.com/en/actions/reference/security/secure-use),
  [environments](https://docs.github.com/en/actions/how-tos/deploy/configure-and-manage-deployments/manage-environments),
  [OIDC](https://docs.github.com/en/actions/concepts/security/openid-connect)
- [crates.io](https://crates.io/docs/trusted-publishing) ·
  [release-plz](https://release-plz.dev/docs/github/quickstart) ·
  [PyPI](https://docs.pypi.org/trusted-publishers/) ·
  [npm](https://docs.npmjs.com/trusted-publishers/)
- [OpenSSF Scorecard](https://github.com/ossf/scorecard)
