#!/usr/bin/env bash
# Sample-profile a single corpus document through pathological_probe.
#
# Usage:
#   scripts/samply-doc.sh <corpus-relative-path> [output-basename]
#
# Example:
#   scripts/samply-doc.sh 001529/files/50685_ruby_67979/50685_ruby_67979.txt
#   # → /tmp/aozora-doc-50685_ruby_67979.json.gz
#
# The script:
# 1. Asserts AOZORA_CORPUS_ROOT is set and the doc exists.
# 2. Asserts /proc/sys/kernel/perf_event_paranoid <= 1 (samply
#    requires perf_event_open access; on most Ubuntu installs the
#    default is 2 and you'll need:
#       echo 1 | sudo tee /proc/sys/kernel/perf_event_paranoid
# 3. Rebuilds pathological_probe with --profile=bench so debug info
#    is preserved. (`cargo run --release` would silently strip it,
#    a foot-gun we hit during the N2 investigation: stack traces
#    came back as raw addresses.)
# 4. Runs samply at 4 kHz, writes a JSON.gz, prints the next steps.

set -euo pipefail

cd "$(dirname "$0")/.."

if [[ "${1-}" == "" || "${1-}" == "-h" || "${1-}" == "--help" ]]; then
    sed -n 's/^# \{0,1\}//p' "$0" | head -25
    exit 1
fi

DOC_REL=$1
BASENAME=${2:-$(basename "${DOC_REL%.txt}")}
OUT=/tmp/aozora-doc-${BASENAME}.json.gz

if [[ -z "${AOZORA_CORPUS_ROOT-}" ]]; then
    echo "error: AOZORA_CORPUS_ROOT not set" >&2
    exit 2
fi

DOC_FULL=${AOZORA_CORPUS_ROOT}/${DOC_REL}
if [[ ! -f $DOC_FULL ]]; then
    echo "error: doc not found at $DOC_FULL" >&2
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

echo ">>> rebuilding pathological_probe with debug info (profile=bench)"
cargo build --profile=bench --example pathological_probe -p aozora-bench 2>&1 \
    | grep -E '^(error|warning|    Finished)' || true

BIN=target/release/examples/pathological_probe
if [[ ! -x $BIN ]]; then
    echo "error: $BIN not built" >&2
    exit 1
fi

echo ">>> samply: doc=$DOC_REL  out=$OUT"
AOZORA_PROBE_DOC=$DOC_REL \
    samply record --save-only --no-open -o "$OUT" -r 4000 -- "$BIN"

echo
echo ">>> done. inspect with:"
echo "    samply load $OUT          # opens local Firefox-Profiler UI"
echo "    gunzip -c $OUT | jq …      # ad-hoc inspection"
