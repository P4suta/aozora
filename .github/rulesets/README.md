# Repository rulesets (as code)

Branch and tag protection for `P4suta/aozora` lives here as version-controlled
JSON, applied through the GitHub REST API. This is the modern replacement for
classic branch-protection rules: reviewable in a PR, reproducible, and
auditable — instead of opaque clicks in the repo settings UI.

GitHub's own ruleset import/export uses exactly this JSON shape
(`POST/PUT /repos/{owner}/{repo}/rulesets`), so these files round-trip with the UI.

## The rulesets

| File | Target | What it enforces | Status |
| --- | --- | --- | --- |
| `main-branch.json` | default branch | required checks (strict), PR (0 approvals, dismiss-stale, conversation resolution), linear history, no force-push, no deletion | **to apply** (migrates the classic protection) |
| `require-signed-commits.json` | all branches | signed commits | already live (ruleset `17766549`) |
| `release-tags.json` | `v*` tags | immutable: no delete / update / force-update | already live (ruleset `15651878`) |

`enforce_admins` from the classic rule maps to an **empty `bypass_actors`** — no
one, including admins, bypasses the ruleset. Commit-signature enforcement is NOT
repeated in `main-branch.json` because `require-signed-commits.json` (`~ALL`)
already covers the default branch; duplicating it would be redundant.

## Migrating the classic branch protection → ruleset

Classic protection and a ruleset **stack** (both enforce; the stricter wins), so
applying the ruleset first leaves no unprotected window. Do it in this order.

```sh
REPO=P4suta/aozora

# 1. Apply the new branch ruleset (POST = create). main now has BOTH the
#    classic protection and this ruleset enforcing — no gap.
gh api "repos/$REPO/rulesets" -X POST --input .github/rulesets/main-branch.json

# 2. Verify it is active and the rules are what we expect.
gh api "repos/$REPO/rulesets" --jq '.[] | {id, name, target, enforcement}'
id=$(gh api "repos/$REPO/rulesets" --jq '.[] | select(.name=="main-branch-protection") | .id')
gh api "repos/$REPO/rulesets/$id" --jq '.rules'

# 3. (Recommended) Confirm a normal PR still requires ci-success + the 3 codeql
#    checks and cannot merge red, and that a direct push to main is rejected.

# 4. Only AFTER the ruleset is confirmed, remove the now-redundant classic rule.
gh api "repos/$REPO/branches/main/protection" -X DELETE
```

To re-sync a ruleset after editing its JSON, use **PUT by id** (POST would create
a duplicate — GitHub allows same-named rulesets):

```sh
gh api "repos/$REPO/rulesets/$id" -X PUT --input .github/rulesets/main-branch.json
```

## Activating release-plz: two App-scoped ruleset changes

`release-plz.yml` runs as a GitHub App (see
`crates/aozora-book/src/contrib/release.md`). Two ruleset changes depend on that
App's numeric **App ID**, which does not exist until the App is created — so they
are **documented here, not committed as `.json`**: shipping a file with a
placeholder integer would be an unappliable landmine, and a real ID can only be
filled at activation time. Apply both as the last steps of bringing release-plz
live (after `RELEASE_PLZ_APP_ID` / `RELEASE_PLZ_APP_PRIVATE_KEY` are set).

Set `APP_ID` to the App's numeric id first:

```sh
REPO=P4suta/aozora
APP_ID=<RELEASE_PLZ_APP_ID>   # the numeric GitHub App ID
```

### 1. Signature bypass on `require-signed-commits`

release-plz pushes the version-bump + CHANGELOG commit **unsigned** (a runner
`git push`, not an API commit) to its `release-plz-*` branch, which
`require-signed-commits` (`~ALL` branches) would reject. Unlike classic "require
signed commits", a ruleset can exempt a specific **Integration (App)** actor via
`bypass_actors`, so we scope the exemption to the one cryptographically-identified
release identity — narrower than a branch-name glob, and the App still cannot
reach `main` (its `pull_request` rule has an empty bypass). The Release PR is
**squash-merged**, so `main` only ever receives GitHub's web-flow-signed merge
commit; the bot's unsigned commits never land there.

```sh
# Re-sync require-signed-commits.json with the App added to bypass_actors:
#   "bypass_actors": [
#     { "actor_id": APP_ID, "actor_type": "Integration", "bypass_mode": "always" }
#   ]
id=$(gh api "repos/$REPO/rulesets" --jq '.[] | select(.name=="require-signed-commits") | .id')
gh api "repos/$REPO/rulesets/$id" -X PUT \
  --input <(jq --argjson app "$APP_ID" \
    '.bypass_actors = [{actor_id: $app, actor_type: "Integration", bypass_mode: "always"}]' \
    .github/rulesets/require-signed-commits.json)
```

### 2. Tag-creation lock (`v*` tags, App-only)

Once release-plz cuts the `v*` tag that fires `release.yml`, restrict `v*` tag
**creation** to the App so a human can no longer hand-push a release tag. Apply
this **last** — applying it before release-plz is live would block the current
manual `git tag v…` flow.

```sh
gh api "repos/$REPO/rulesets" -X POST --input <(jq -n --argjson app "$APP_ID" '{
  name: "release-tags-app-only",
  target: "tag",
  enforcement: "active",
  conditions: { ref_name: { include: ["refs/tags/v*"], exclude: [] } },
  rules: [{ type: "creation" }],
  bypass_actors: [{ actor_id: $app, actor_type: "Integration", bypass_mode: "always" }]
}')
```

Both use the same App id → one coherent "the release-plz App is our trusted
release identity" model across signing and tagging.
