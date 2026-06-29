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

`release-plz.yml` runs as the **release-plz GitHub App** (App ID `4177429`; see
`crates/aozora-book/src/contrib/release.md`). Two rulesets grant that App the
access release-plz needs; both are committed here **with the App baked in**, so
applying each is a single `gh api … --input <file>` — no jq, no placeholders.

> A maintainer runs these two commands: the auto-mode agent is intentionally
> blocked from mutating rulesets via `gh api`.

### 1. Signature bypass on `require-signed-commits` — apply during activation

release-plz pushes the version-bump + CHANGELOG commit **unsigned** (a runner
`git push`, not an API commit) to its `release-plz-*` branch, which
`require-signed-commits` (`~ALL` branches) would reject. A ruleset can exempt a
specific **Integration (App)** actor via `bypass_actors`, scoping the exemption
to the one cryptographically-identified release identity — narrower than a
branch-name glob, and the App still cannot reach `main` (its `pull_request` rule
has an empty bypass). Release PRs are **squash-merged**, so `main` only ever
receives GitHub's web-flow-signed merge commit.

`require-signed-commits.json` already lists the App in `bypass_actors`, so
re-sync the live ruleset (id `17766549`) straight from the file:

```sh
gh api repos/P4suta/aozora/rulesets/17766549 -X PUT \
  --input .github/rulesets/require-signed-commits.json
gh api repos/P4suta/aozora/rulesets/17766549 --jq '.bypass_actors'   # verify
```

### 2. Tag-creation lock (`v*` tags, App-only) — apply LAST

Once release-plz cuts the `v*` tag that fires `release.yml`, restrict `v*` tag
**creation** to the App so a human can no longer hand-push a release tag. Apply
this **after the first successful release** — applying it earlier would block the
current manual `git tag v…` flow.

```sh
gh api repos/P4suta/aozora/rulesets -X POST \
  --input .github/rulesets/release-tags-creation.json
```

Both use the same App id (`4177429`) → one coherent "the release-plz App is our
trusted release identity" model across signing and tagging.
