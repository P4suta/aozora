#!/usr/bin/env bash

set -euo pipefail

remote="${1:-origin}"
zero_sha="0000000000000000000000000000000000000000"

branch="$(git rev-parse --abbrev-ref HEAD)"
local_sha="$(git rev-parse HEAD)"

remote_sha="$(git rev-parse --verify --quiet "refs/remotes/${remote}/${branch}" || echo "$zero_sha")"

if [ "$remote_sha" = "$zero_sha" ]; then
  range=$(git rev-list "$local_sha" --not --remotes="$remote")
else
  range=$(git rev-list "${remote_sha}..${local_sha}")
fi

exit_code=0
for commit in $range; do
  if ! git verify-commit "$commit" >/dev/null 2>&1; then
    echo "::error:: unsigned commit cannot be pushed: $commit" >&2
    git log -1 --pretty=format:"  %h %s%n  author: %an <%ae>%n" "$commit" >&2
    exit_code=1
  fi
done

if [ "$exit_code" -ne 0 ]; then
  cat >&2 <<'MSG'

::notice:: push refused. To sign these commits:

  # last commit only
  git commit --amend -S --no-edit

  # rewrite a range
  git rebase --exec 'git commit --amend --no-edit -S' <base>

If signing is intentionally not configured for this repo, override with
`git push --no-verify` (but Claude Code is denied that flag by default).
MSG
fi

exit $exit_code
