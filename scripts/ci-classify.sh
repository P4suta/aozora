#!/usr/bin/env bash
# Classify a list of changed paths (one per line, on stdin) into the
# coarse gate categories `just ci-parallel`'s opt-in fast mode uses
# (`AOZORA_CI_FAST=1`; #81, ADR-0007). Prints the distinct touched
# categories on one line, space-separated:
#
#   code  — anything that can affect the Rust build / test / lint /
#           conformance / render output. The DEFAULT bucket: any path that
#           is not provably play / book / infra lands here, so the heavy
#           gates run unless the change is confined to an isolated area.
#   play  — playground/** (the isolated Solid + Vite + WASM frontend).
#   book  — handbook / ADR markdown (docs/**, crates/aozora-book/**) — book
#           content with no Rust-doctest impact. (A Rust *doc comment* lives
#           in crates/**/src/*.rs and is therefore `code`, not `book`.)
#   infra — the gate definitions themselves (Justfile / lefthook / CI /
#           docker). When present the caller MUST run the full matrix: the
#           skip map itself may be changing, so trusting it would be circular.
#
# Conservative by construction — it can only ever cause MORE gates to run
# than strictly necessary, never fewer. The cloud CI runs the full matrix
# regardless, as the backstop.
set -euo pipefail

code=0 play=0 book=0 infra=0
while IFS= read -r f; do
    [[ -z "$f" ]] && continue
    case "$f" in
        playground/*) play=1 ;;
        Justfile | lefthook.yml | .github/* | scripts/* | compose*.y*ml | docker/* | *Dockerfile*) infra=1 ;;
        docs/* | crates/aozora-book/*) book=1 ;;
        *) code=1 ;;
    esac
done

out=""
[[ $code -eq 1 ]] && out+="code "
[[ $play -eq 1 ]] && out+="play "
[[ $book -eq 1 ]] && out+="book "
[[ $infra -eq 1 ]] && out+="infra "
echo "${out% }"
