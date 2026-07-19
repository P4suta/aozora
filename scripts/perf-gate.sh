#!/usr/bin/env bash
set -euo pipefail

baseline=crates/aozora-bench/perf-baseline.tsv
binary=/cargo/target/release/examples/perf_gate
limit_percent=10
cases=(parse-tosa parse-matoi parse-peter html-tosa html-matoi parse-dense)

cargo build --release -p aozora-bench --example perf_gate

declare -A expected
while IFS=$'\t' read -r name instructions; do
    [[ -n "$name" ]] && expected[$name]=$instructions
done < "$baseline"

failed=0
for name in "${cases[@]}"; do
    out="/tmp/aozora-callgrind-$name.out"
    valgrind --quiet --tool=callgrind --callgrind-out-file="$out" "$binary" "$name" >/dev/null
    actual=$(awk '/^summary:/ { print $2 }' "$out")
    reference=${expected[$name]:-}
    if [[ -z "$actual" || -z "$reference" ]]; then
        echo "$name: missing actual or baseline instruction count" >&2
        failed=1
        continue
    fi
    ceiling=$((reference + reference * limit_percent / 100))
    printf '%-14s %10d instructions (baseline %10d, ceiling %10d)\n' \
        "$name" "$actual" "$reference" "$ceiling"
    if (( actual > ceiling )); then
        echo "::error title=perf-gate::$name regressed beyond ${limit_percent}%" >&2
        failed=1
    fi
done

exit "$failed"
