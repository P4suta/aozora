#!/usr/bin/env bash
# Sample-profile the parser hot path across the full corpus via
# throughput_by_class. The profile measures the parser; the corpus
# load (Shift-JIS decode + bucketing) is excluded by re-running the
# parse pass AOZORA_PROFILE_REPEAT times so samples land in
# lex_into_arena rather than in glibc syscalls.
#
# Usage:
#   scripts/samply-corpus.sh [repeat_count]
#
# Example:
#   scripts/samply-corpus.sh 5
#   # → /tmp/aozora-corpus-<timestamp>.json.gz
#
# Why not just point samply at one `cargo run`?
#   - samply needs debug info; `cargo run --release` strips it.
#     We use --profile=bench (release + debug=1 + strip=none).
#   - samply needs perf_event_paranoid <= 1.
#   - The raw 3.85 s parser pass on a corpus of 17 K docs is mostly
#     spent reading those docs from disk and decoding Shift-JIS.
#     Repeating the parse loop K times (load happens once, parse
#     happens K times) tilts the profile so the parser dominates.

set -euo pipefail

cd "$(dirname "$0")/.."

if [[ "${1-}" == "-h" || "${1-}" == "--help" ]]; then
    sed -n 's/^# \{0,1\}//p' "$0" | head -25
    exit 1
fi

REPEAT=${1:-5}
TS=$(date +%Y%m%d-%H%M%S)
OUT=/tmp/aozora-corpus-${TS}.json.gz

if [[ -z "${AOZORA_CORPUS_ROOT-}" ]]; then
    echo "error: AOZORA_CORPUS_ROOT not set" >&2
    exit 2
fi

PARANOID=$(cat /proc/sys/kernel/perf_event_paranoid)
if (( PARANOID > 1 )); then
    cat >&2 <<EOF
error: /proc/sys/kernel/perf_event_paranoid = $PARANOID (need <= 1 for samply)
       Run once per boot:
           echo 1 | sudo tee /proc/sys/kernel/perf_event_paranoid
EOF
    exit 2
fi

echo ">>> rebuilding throughput_by_class with debug info (profile=bench)"
cargo build --profile=bench --example throughput_by_class -p aozora-bench 2>&1 \
    | grep -E '^(error|warning|    Finished)' || true

BIN=target/release/examples/throughput_by_class
if [[ ! -x $BIN ]]; then
    echo "error: $BIN not built" >&2
    exit 1
fi

echo ">>> samply: repeat=${REPEAT}  out=$OUT"
AOZORA_PROFILE_REPEAT=$REPEAT \
    samply record --save-only --no-open -o "$OUT" -r 4000 -- "$BIN"

echo
echo ">>> done. inspect with:"
echo "    samply load $OUT          # opens local Firefox-Profiler UI"
