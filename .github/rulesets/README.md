# Repository rulesets

These JSON files mirror the active rulesets for `P4suta/aozora` in GitHub's import/export shape.
They omit server-managed metadata and can be imported through the settings UI or applied through the REST API.

| File | Live name | Target | Protection |
| --- | --- | --- | --- |
| `main-branch.json` | `main` | default branch | Strict `required` check, squash-only pull requests, resolved review threads, signed commits, linear history, no deletion or force-push |
| `release-tags.json` | `release tags` | `refs/tags/v*` | No deletion, update, or force-update |

Both rulesets are active and have empty `bypass_actors`, including for administrators and the release-plz App.
Signed commits are required on the default branch by `main`.
The `required` check gates merges; `release-ready` qualifies the exact commit for publication.

## Compare with GitHub

List the live rulesets and identify each by its name and target:

```sh
REPO=P4suta/aozora
gh api --method GET "repos/$REPO/rulesets" --jq '.[] | {id, name, target, enforcement}'
```

Compare each committed file with the matching ruleset's full definition, retaining only importable fields:

```sh
for file in .github/rulesets/*.json; do
  id=$(gh api --method GET "repos/$REPO/rulesets" | jq -er --slurpfile committed "$file" \
    '.[] | select(.name == $committed[0].name and .target == $committed[0].target) | .id') || exit 1
  diff -u <(jq -S . "$file") \
    <(gh api --method GET "repos/$REPO/rulesets/$id" | jq -S '{name, target, enforcement, conditions, rules, bypass_actors}') || exit 1
done
```

## Re-apply a reviewed definition

Use the matching live ID from the comparison and update the existing ruleset with `PUT`:

```sh
gh api --method PUT "repos/$REPO/rulesets/$id" --input "$file"
```

Set `id` and `file` to the intended pair before running the update, then repeat the comparison.
Creating with `POST` would add another ruleset, including when its name already exists.
Run `cargo run -p aozora-xtask -q -- release check` in the pinned mise environment to verify the committed merge gate and tag-immutability rules.

## Optional tag-creation restriction

The committed tag ruleset permits creation and makes existing release tags immutable.
After observing release-plz create a release tag successfully, a maintainer may add a separate `creation` ruleset for `refs/tags/v*` with only the release-plz App as an `Integration` bypass actor.
Use the numeric App ID for that optional rule, rather than the Client ID used for authentication.
Keep the existing immutable-tag ruleset's bypass list empty.
