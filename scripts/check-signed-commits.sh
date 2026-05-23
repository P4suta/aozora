#!/usr/bin/env bash
# Refuse to push if any commit in the range about to be pushed is unsigned.
#
# Multi-layer signing defense (all retained intentionally):
#   - commit-time:  `.git/hooks/post-commit` re-amends unsigned commits
#                    via the SSH/GPG signer; rolls back on failure
#   - push-time:    this script, invoked from lefthook's pre-push, blocks
#                    any unsigned commit that slipped through (e.g. merges
#                    from another machine or `--no-verify` on the local hook)
#   - server-side:  GitHub repository ruleset "Require signed commits"
#                    is the final authority
#
# Bypass surface: `git push --no-verify` skips pre-push. Claude Code's
# `settings.json` deny list excludes that flag for the agent case.
#
# Invocation
# ----------
# Lefthook does not forward git's stdin to commands by default (pre-push
# normally feeds `<local_ref> <local_sha> <remote_ref> <remote_sha>` lines
# on stdin). To stay robust regardless of how this hook is wired, we
# rederive the push range from git's own refs:
#
#   1. take the current branch's HEAD as the "to" end of the range
#   2. take the corresponding remote-tracking ref as the "from" end
#      (or fall back to "all commits not reachable from any remote ref"
#      when the branch is brand-new on the remote)
#
# Manual use: `scripts/check-signed-commits.sh [remote]` — handy for
# auditing a branch before publishing.

set -euo pipefail

remote="${1:-origin}"
zero_sha="0000000000000000000000000000000000000000"

branch="$(git rev-parse --abbrev-ref HEAD)"
local_sha="$(git rev-parse HEAD)"

# The remote-tracking ref may not exist yet (branch is new on the remote).
remote_sha="$(git rev-parse --verify --quiet "refs/remotes/${remote}/${branch}" || echo "$zero_sha")"

if [ "$remote_sha" = "$zero_sha" ]; then
  # New branch on remote: check commits reachable from HEAD but not
  # from any existing remote ref. Avoids re-verifying history that
  # already lives on the remote.
  range=$(git rev-list "$local_sha" --not --remotes="$remote")
else
  # Existing branch: check the new commits only.
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
