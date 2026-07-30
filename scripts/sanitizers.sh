#!/usr/bin/env bash
# Run aozora workspace tests under a sanitiser.
#
# Usage:
#   scripts/sanitizers.sh miri        [--filter <pat>]   # MIR interpreter (UB detection)
#   scripts/sanitizers.sh tsan        [--filter <pat>]   # ThreadSanitizer (data race)
#   scripts/sanitizers.sh asan        [--filter <pat>]   # AddressSanitizer (memory safety)
#
# Why this script exists
#   Concurrency bugs that escape proptest/stress tests show up as
#   data races (TSan), use-after-free (ASan), or undefined behaviour
#   (Miri). These are nightly-only Rust capabilities that need a
#   one-line invocation; this wrapper centralises the env var dance
#   so the developer doesn't need to remember it.
#
# Notes
#   - Each sanitiser uses the date-pinned nightly from mise.toml.
#   - Miri rejects most C dependencies and is slow; expect 10-100x
#     slower than `cargo test`. Use --filter to scope.
#   - TSan requires `RUSTFLAGS=-Zsanitizer=thread` and a panic=abort
#     profile. The script forces `--target` to the host triple
#     because sanitisers don't work with cross-compilation.
#   - Scheduled CI provides early warning; release-ready runs ASan and TSan on
#     the exact commit whose artifacts it qualifies.

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
        # `Z build-std=std,test` is required because the sanitiser
        # needs to instrument std as well. RUSTFLAGS reaches both
        # the binary and std rebuild.
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
