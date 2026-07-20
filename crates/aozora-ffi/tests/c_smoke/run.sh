#!/usr/bin/env bash
# C ABI smoke test driver.
#
# Builds the aozora-ffi cdylib, compiles the C smoke harness against
# it, runs it, and exits non-zero on any failure. Intended to be
# called from CI and from the workspace's `just bench-ffi` recipe
# once that lands.

set -euo pipefail

cd "$(dirname "$0")/../../../.."

# 1. Build the cdylib in release mode (so it carries the workspace
#    LTO + opt-level=3 settings the FFI consumer actually links).
cargo build --release -p aozora-ffi

RELEASE_DIR="${CARGO_TARGET_DIR:-target}/release"
CDYLIB="$RELEASE_DIR/libaozora_ffi.so"
if [[ ! -f "$CDYLIB" ]]; then
    echo "expected $CDYLIB to exist after build" >&2
    exit 2
fi

SMOKE_C="crates/aozora-ffi/tests/c_smoke/smoke.c"
SMOKE_BIN="$RELEASE_DIR/aozora_ffi_smoke"

# 2. Compile the C harness, linking against the cdylib. `-I target/release`
#    is where build.rs mirrors the cbindgen-generated `aozora.h` that
#    smoke.c includes.
gcc -O2 -Wall -Wextra -I "$RELEASE_DIR" -o "$SMOKE_BIN" "$SMOKE_C" \
    -L "$RELEASE_DIR" -laozora_ffi

# 3. Run with the cdylib's directory on LD_LIBRARY_PATH so dlopen
#    finds it without needing a system install.
LD_LIBRARY_PATH="$RELEASE_DIR:${LD_LIBRARY_PATH:-}" "$SMOKE_BIN"
