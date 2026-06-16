#!/usr/bin/env bash
# Vendor the conformance vectors from the sibling aozora-notation-spec
# repository into this parser tree.
#
# The specification is the SINGLE SOURCE OF TRUTH for the conformance
# corpus. We vendor a copy here (rather than a git submodule / crates.io
# pin) so that the in-container `just conformance` gate and cloud CI can
# run the vectors without network access or a checked-out sibling repo.
# The vendored copy under crates/aozora-conformance/spec-vectors/ MUST
# NOT be hand-edited — edit the spec, then re-run this script and commit
# the diff. `scripts/verify-spec-vectors.sh` (pre-push) guards the
# vendored copy against drifting from the spec.
#
# Override the spec location with AOZORA_SPEC_REPO (default: the sibling
# ../aozora-notation-spec relative to this repo's root).
#
# Host-side only: reaches outside the /workspace bind mount, so it runs
# directly on the host, never inside the dev container.

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

spec="${AOZORA_SPEC_REPO:-../aozora-notation-spec}"
dest="crates/aozora-conformance/spec-vectors"

if [ ! -d "$spec/conformance/vectors" ]; then
  echo "sync-spec-vectors: spec repo not found at '$spec' (set AOZORA_SPEC_REPO)" >&2
  exit 1
fi

# Replace the vendored vectors + schema wholesale so deletions in the
# spec propagate (a stale leftover vector would otherwise linger). The
# README is parser-repo-owned and is left untouched.
rm -rf "$dest/vectors" "$dest/schema"
mkdir -p "$dest/vectors" "$dest/schema"
cp -R "$spec/conformance/vectors/." "$dest/vectors/"
cp "$spec/conformance/schema/vector.schema.json" "$dest/schema/vector.schema.json"
cp "$spec/conformance/RUNNER.md" "$dest/RUNNER.md"

count="$(find "$dest/vectors" -name vector.json | wc -l | tr -d ' ')"
echo "sync-spec-vectors: vendored $count vector(s) from '$spec' → $dest"
