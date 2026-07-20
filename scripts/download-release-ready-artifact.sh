#!/usr/bin/env bash
set -euo pipefail

artifact=${1:?artifact name required}
destination=${2:?destination required}
sha=${3:-${GITHUB_SHA:?GITHUB_SHA required}}

run_id=$(
    gh run list \
        --workflow release-ready.yml \
        --commit "$sha" \
        --status success \
        --limit 20 \
        --json databaseId,headSha,conclusion \
        --jq "map(select(.headSha == \"$sha\" and .conclusion == \"success\"))[0].databaseId"
)
if [[ -z "$run_id" || "$run_id" == null ]]; then
    echo "::error title=release-ready::no successful release-ready run for $sha" >&2
    exit 1
fi

mkdir -p "$destination"
gh run download "$run_id" --name "$artifact" --dir "$destination"
