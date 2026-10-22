#!/usr/bin/env bash

set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/sanitizers.sh <miri|tsan|asan> [--filter <pattern>]

Examples:
  scripts/sanitizers.sh miri
  scripts/sanitizers.sh miri --filter incremental
  scripts/sanitizers.sh tsan --filter concurrent
  scripts/sanitizers.sh asan
EOF
    exit 1
}

if [[ $# -lt 1 ]]; then
    usage
fi

mode="$1"
shift

filter=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --filter)
            filter="$2"
            shift 2
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo "unknown arg: $1" >&2
            usage
            ;;
    esac
done

nightly="nightly-2026-07-15"

host_triple() {
    rustc -vV | sed -n 's/^host: //p'
}

case "$mode" in
    miri)
        echo ">> running cargo +$nightly miri test${filter:+ with filter: $filter}" >&2
        if [[ -n "$filter" ]]; then
            cargo "+$nightly" miri test --workspace -- "$filter"
        else
            cargo "+$nightly" miri test --workspace
        fi
        ;;
    tsan)
        target="$(host_triple)"
        echo ">> running ThreadSanitizer (target=$target)" >&2
        export RUSTFLAGS="-Zsanitizer=thread"
        export RUSTDOCFLAGS="-Zsanitizer=thread"
        if [[ -n "$filter" ]]; then
            cargo "+$nightly" test \
                -Z build-std \
                --target "$target" \
                --workspace \
                -- "$filter"
        else
            cargo "+$nightly" test \
                -Z build-std \
                --target "$target" \
                --workspace
        fi
        ;;
    asan)
        target="$(host_triple)"
        echo ">> running AddressSanitizer (target=$target)" >&2
        export RUSTFLAGS="-Zsanitizer=address"
        export RUSTDOCFLAGS="-Zsanitizer=address"
        if [[ -n "$filter" ]]; then
            cargo "+$nightly" test \
                -Z build-std \
                --target "$target" \
                --workspace \
                -- "$filter"
        else
            cargo "+$nightly" test \
                -Z build-std \
                --target "$target" \
                --workspace
        fi
        ;;
    *)
        echo "unknown mode: $mode" >&2
        usage
        ;;
esac
