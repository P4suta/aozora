#!/usr/bin/env bash
set -euo pipefail

corpus=${AOZORA_CORPUS_ROOT:?AOZORA_CORPUS_ROOT must name a corpus directory inside the workspace}
test -d "$corpus"
test -n "$(find "$corpus" -type f -name '*.txt' -print -quit)"

target=${CARGO_TARGET_DIR:-target}
profile_dir="$target/pgo-data"
profile_data="$target/aozora.profdata"
rm -rf "$profile_dir"
mkdir -p "$profile_dir"

export RUSTFLAGS="-Cprofile-generate=$profile_dir"
export LLVM_PROFILE_FILE="$profile_dir/aozora-%p-%m.profraw"
cargo build --locked --profile dist -p aozora-cli

binary="$target/dist/aozora"
training_count=0
while IFS= read -r source; do
    "$binary" check --format json "$source" >/dev/null
    "$binary" render "$source" >/dev/null
    training_count=$((training_count + 1))
done < <(find "$corpus" -type f -name '*.txt' -print | sort | sed -n '1,120p')
test "$training_count" -gt 0

host=$(rustc -vV | awk '/^host:/ { print $2 }')
llvm_profdata="$(rustc --print sysroot)/lib/rustlib/$host/bin/llvm-profdata"
llvm_readobj="$(dirname "$llvm_profdata")/llvm-readobj"
"$llvm_profdata" merge -o "$profile_data" "$profile_dir"/*.profraw
"$llvm_profdata" show --all-functions --counts "$profile_data" |
    grep -E 'Functions shown: [1-9]' >/dev/null

unset LLVM_PROFILE_FILE
export RUSTFLAGS="-Cprofile-use=$profile_data -Cllvm-args=-pgo-warn-missing-function"
cargo build --locked --profile dist -p aozora-cli

if "$llvm_readobj" --sections "$binary" |
    grep -E '(__llvm_prf|\.lprf)' >/dev/null; then
    echo "final CLI is still profile-instrumented" >&2
    exit 1
fi

mkdir -p target/pgo
cp "$binary" target/pgo/aozora
