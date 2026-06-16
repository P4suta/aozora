#!/usr/bin/env bash
# Verify the vendored conformance vectors match the sibling spec repo.
#
# This is the host-side half of the "spec is master" loop: it fails the
# push if crates/aozora-conformance/spec-vectors/ has drifted from
# ../aozora-notation-spec (i.e. someone hand-edited the vendored copy, or
# forgot to re-run `just sync-spec-vectors` after changing the spec).
#
# Skip-if-absent: cloud CI and the dev container have no sibling spec
# checkout, so this exits 0 there — in those environments the vendored
# copy IS authoritative (the parser is tested against it), and the
# vendored==spec invariant is enforced on the developer host at push time.
#
# Override the spec location with AOZORA_SPEC_REPO.

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

spec="${AOZORA_SPEC_REPO:-../aozora-notation-spec}"
dest="crates/aozora-conformance/spec-vectors"

if [ ! -d "$spec/conformance/vectors" ]; then
  echo "verify-spec-vectors: spec repo not present at '$spec' — skipping (vendored copy is authoritative here)"
  exit 0
fi

fail=0
diff -r "$spec/conformance/vectors" "$dest/vectors" || fail=1
diff "$spec/conformance/schema/vector.schema.json" "$dest/schema/vector.schema.json" || fail=1
diff "$spec/conformance/RUNNER.md" "$dest/RUNNER.md" || fail=1

if [ "$fail" -ne 0 ]; then
  echo "::error:: vendored spec vectors drift from '$spec'. Run 'just sync-spec-vectors' and commit the diff." >&2
  exit 1
fi

echo "verify-spec-vectors: vendored copy matches '$spec' ✔"
