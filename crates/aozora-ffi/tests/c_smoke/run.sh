#!/usr/bin/env bash

set -euo pipefail

cd "$(dirname "$0")/../../../.."

# `just smoke-ffi` uses release settings to match the library linked by C ABI consumers.
cargo build --release -p aozora-ffi

RELEASE_DIR="${CARGO_TARGET_DIR:-target}/release"
CDYLIB="$RELEASE_DIR/libaozora_ffi.so"
if [[ ! -f "$CDYLIB" ]]; then
    echo "expected $CDYLIB to exist after build" >&2
    exit 2
fi

SMOKE_C="crates/aozora-ffi/tests/c_smoke/smoke.c"
SMOKE_BIN="$RELEASE_DIR/aozora_ffi_smoke"

# build.rs mirrors the generated aozora.h into the release directory for this harness.
gcc -O2 -Wall -Wextra -I "$RELEASE_DIR" -o "$SMOKE_BIN" "$SMOKE_C" \
    -L "$RELEASE_DIR" -laozora_ffi

LD_LIBRARY_PATH="$RELEASE_DIR:${LD_LIBRARY_PATH:-}" "$SMOKE_BIN"
