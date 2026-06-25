# Release secrets & Trusted Publishing

This is the operational runbook for aozora's release credentials. It
follows the **official guidance** of each platform — GitHub Actions
security hardening, and the Trusted Publishing (OIDC) flows of crates.io,
PyPI, and npm — rather than any bespoke scheme. See the [References](#references)
at the bottom for every primary source.

The goal: **no long-lived publish token sits in the repository.** Where
the registry supports OIDC Trusted Publishing we mint a short-lived token
at publish time; where it does not (VS Code Marketplace, Open VSX) the
token lives as a GitHub *Environment* secret behind an approval gate, not
as a repository secret.

## Security model

Every workflow that can publish or create a release splits into an
**ungated `build` job** (no credentials) and a **gated `publish` job**
that runs in the `release` GitHub Environment:

- The publish job pauses for **required-reviewer approval** before it is
  sent to a runner. Per GitHub's docs, *"a job cannot access environment
  secrets until one of the required reviewers approves it"* and *"any
  protection rules … must pass before a job referencing the environment
  is sent to a runner."* So neither environment secrets **nor** the OIDC
  token are reachable until you approve.
- The environment restricts **deployment branches and tags** to `main`
  (dispatch-triggered publishes run from `main`) plus the `v*` /
  `vscode-v*` release tags. An OIDC token minted under the environment
  carries `…:environment:release` in its `sub` claim, so the registry's
  trusted-publisher rule can require that environment.
- `dry_run: true` is the default on every manual workflow, so a stray
  "Run workflow" click only runs the ungated dry-run.

## One-time setup

### 1. Create the `release` environment

Already scriptable with `gh` (repo admin required). Settings →
Environments → `release`, or:

```sh
# required reviewer = the maintainer; restrict to release refs
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

> A workflow that names an environment which does **not** exist yet
> auto-creates it **without** protection rules on first run. Always
> create the environment (above) **before** the first gated run.

### 2. crates.io — Trusted Publishing

crates.io does **not** allow Trusted Publishing for a crate that does not
exist yet: *"initial publish requires an API token."* So the very first
publish of the 13 brand-new `aozora*` crates is a one-time token
bootstrap; every release afterwards is tokenless.

**Bootstrap (once):**

1. Create a crates.io API token with **both** `publish-new` and
   `publish-update` scopes.
2. Add it as the `CARGO_TOKEN` **Environment** secret on `release`
   (`gh secret set CARGO_TOKEN --env release`).
3. Run the ladder with OIDC disabled and approve the `publish` job:
   ```sh
   gh workflow run publish-crates.yml -f dry_run=false -f use_oidc=false
   ```
   crates.io throttles new crates (burst 5, then ~1 / 10 min); the
   workflow is resumable and rate-limit aware, so re-run until all 13 are
   live (already-published versions are skipped).

**Switch to OIDC (after the crates exist):**

4. For **each** of the 13 published crates: crates.io → the crate →
   Settings → Trusted Publishing → Add → GitHub, with
   - Repository owner: `P4suta`
   - Repository name: `aozora`
   - Workflow filename: `publish-crates.yml`
   - Environment: `release`
5. Delete the `CARGO_TOKEN` environment secret
   (`gh secret delete CARGO_TOKEN --env release`). Steady-state releases
   now run `gh workflow run publish-crates.yml -f dry_run=false`
   (`use_oidc` defaults to true); `rust-lang/crates-io-auth-action` mints
   a 30-minute token that is auto-revoked when the job ends.

### 3. PyPI — Trusted Publishing (tokenless from day one)

PyPI supports a **"pending publisher"**, so a brand-new project needs no
token bootstrap.

1. PyPI → Your projects → Publishing → add a **pending publisher**:
   - PyPI Project Name: the distribution name of `aozora-py`
   - Owner: `P4suta`, Repository: `aozora`
   - Workflow name: `publish-pypi.yml`
   - Environment name: `release`
2. Publish promptly — a pending publisher does **not** reserve the name
   until first use, so register-then-publish back to back.
   ```sh
   gh workflow run publish-pypi.yml -f dry_run=false   # then approve
   ```
   The first OIDC upload creates the project and promotes the pending
   publisher to a normal one. No `PYPI_TOKEN` is ever stored.

### 4. npm — Trusted Publishing

Like crates.io, npm requires the package to **already exist** before a
trusted publisher can be configured (*"Package must exist"*), and the
account must have **2FA enabled**. So npm is a one-time token bootstrap.

> Trusted Publishing needs **npm CLI ≥ 11.5.1** and **Node ≥ 22.14.0**;
> the workflow upgrades npm on the runner.

**Bootstrap (once):**

1. Create an npm **granular access / automation** token that can publish
   `aozora-wasm`.
2. Add it as the `NPM_TOKEN` **Environment** secret on `release`.
3. Bootstrap and approve:
   ```sh
   gh workflow run publish-npm.yml -f dry_run=false -f use_oidc=false
   ```

**Switch to OIDC:**

4. npmjs.com → the package → Settings → Trusted Publisher → GitHub
   Actions, with owner/repo, workflow `publish-npm.yml`, and
   Environment name `release`.
5. Delete the `NPM_TOKEN` environment secret. Steady-state releases run
   `gh workflow run publish-npm.yml -f dry_run=false`; npm attaches a
   provenance attestation automatically (no `--provenance` flag needed).

### 5. VS Code Marketplace & Open VSX (no OIDC)

These have no OIDC publishing, so the tokens stay — but as **Environment**
secrets behind the approval gate, never as repository secrets:

```sh
gh secret set VSCE_PAT --env release   # Azure DevOps PAT (Marketplace)
gh secret set OVSX_PAT --env release   # Open VSX token (optional)
```

The `release-vscode.yml` publish job already references these names; they
now resolve from the environment.

### 6. Delete the old repository secrets

Repository-level secrets weaken the environment gate (any workflow run
can read them), so once the values live on the `release` environment,
remove the repo copies:

```sh
gh secret delete VSCE_PAT
gh secret delete OVSX_PAT
gh secret delete CARGO_TOKEN   # if it ever existed at repo level
gh secret delete NPM_TOKEN     # ditto
```

## Routine release (steady state)

1. Tag / dispatch as described in [Release process](release.md).
2. For dispatch publishes, run with `dry_run=false`; for tag-driven
   releases (`v*`, `vscode-v*`) the push triggers the workflow.
3. **Approve** the `publish` job in the run's environment-deployment
   prompt (Actions → the run → Review deployments).
4. Done — no token handling.

## Verification

```sh
# environment holds the non-OIDC tokens; repo does NOT
gh secret list --env release
gh secret list                       # VSCE_PAT/OVSX_PAT must be absent

# the deployment policy only allows release refs
gh api repos/P4suta/aozora/environments/release/deployment-branch-policies

# build provenance (SLSA Build L2) on a released artefact
gh attestation verify <archive> --repo P4suta/aozora
```

A green `Scorecard supply-chain security` workflow (code scanning) keeps
the Token-Permissions and Pinned-Dependencies posture from regressing.

## References

- GitHub — Security hardening for GitHub Actions:
  <https://docs.github.com/en/actions/security-for-github-actions/security-guides/security-hardening-for-github-actions>
- GitHub — Managing environments / deployment protection rules:
  <https://docs.github.com/en/actions/how-tos/deploy/configure-and-manage-deployments/manage-environments>
- GitHub — Security hardening with OpenID Connect:
  <https://docs.github.com/en/actions/concepts/security/openid-connect>
- crates.io — Trusted Publishing: <https://crates.io/docs/trusted-publishing>
  · `rust-lang/crates-io-auth-action`: <https://github.com/rust-lang/crates-io-auth-action>
- PyPI — Trusted Publishers: <https://docs.pypi.org/trusted-publishers/>
- npm — Trusted Publishers: <https://docs.npmjs.com/trusted-publishers>
- OpenSSF Scorecard: <https://github.com/ossf/scorecard>
