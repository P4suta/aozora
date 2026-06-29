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

## Future: release-plz tag gate

When release-plz is activated (it cuts the `v*` tag that fires `release.yml`), add
a **tag-creation restriction** so only the release-plz GitHub App may create
`v*` tags — a human can no longer hand-push a release tag. That is a `creation`
rule on `refs/tags/v*` with the App in `bypass_actors`:

```jsonc
// add as .github/rulesets/release-tags-creation.json (NOT applied yet — it would
// block the current manual `git tag v…` release flow until release-plz is live)
{
  "name": "release-tags-app-only",
  "target": "tag",
  "enforcement": "active",
  "conditions": { "ref_name": { "include": ["refs/tags/v*"], "exclude": [] } },
  "rules": [{ "type": "creation" }],
  "bypass_actors": [
    { "actor_id": <RELEASE_PLZ_APP_ID>, "actor_type": "Integration", "bypass_mode": "always" }
  ]
}
```
