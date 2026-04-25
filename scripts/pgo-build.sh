#!/usr/bin/env bash
#
# PGO (Profile-Guided Optimisation) release build pipeline for the
# aozora workspace.
#
# Three phases:
#   1. instrumented build  — compile aozora-cli + aozora-bench with
#                            instrumentation that records hot paths
#   2. profile collection  — run the instrumented binary against the
#                            full Aozora corpus to gather a representative
#                            profile
#   3. optimised rebuild   — re-link with the collected profile, baking
#                            the hot-path layout into the final binary
#
# Optional fourth phase (BOLT post-link, Linux x86_64 only):
#   4. llvm-bolt           — apply binary post-link layout optimisation
#                            on top of the PGO output
#
# Expected gain: 10-15% additional throughput per LLVM project's
# published numbers; aozora-specific measurement is part of this
# script's reporting.
#
# Requirements (verified at the top of the script):
#   - cargo-pgo  — install via `cargo install cargo-pgo`
#   - llvm-tools-preview — install via `rustup component add llvm-tools-preview`
#   - AOZORA_CORPUS_ROOT environment variable pointing at the
#     extracted Aozora text corpus
#   - llvm-bolt — for the optional BOLT phase, install via the
#     `llvm-bolt` package on Debian/Ubuntu

set -euo pipefail

cd "$(dirname "$0")/.."

# ----------------------------------------------------------------------
# Phase 0 — preflight
# ----------------------------------------------------------------------

require() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "FATAL: required tool '$1' not found in PATH" >&2
        echo "       installation hint: $2" >&2
        exit 2
    fi
}

require cargo "rustup (https://rustup.rs/)"
require cargo-pgo "cargo install cargo-pgo"

if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
    echo "FATAL: AOZORA_CORPUS_ROOT not set; PGO needs a corpus to profile" >&2
    echo "       set it to the extracted aozorabunko_text root, e.g." >&2
    echo "       export AOZORA_CORPUS_ROOT=~/aozora-corpus/aozorabunko_text-master/cards" >&2
    exit 2
fi

if [[ ! -d "$AOZORA_CORPUS_ROOT" ]]; then
    echo "FATAL: AOZORA_CORPUS_ROOT='$AOZORA_CORPUS_ROOT' is not a directory" >&2
    exit 2
fi

echo "==> Preflight checks passed"
echo "    corpus root: $AOZORA_CORPUS_ROOT"
echo "    cargo-pgo:   $(cargo pgo --version 2>/dev/null | head -1)"

# ----------------------------------------------------------------------
# Phase 1 — instrumented build
# ----------------------------------------------------------------------

echo ""
echo "==> Phase 1: instrumented build"
cargo pgo build -- -p aozora-cli -p aozora-parser --example profile_corpus

# ----------------------------------------------------------------------
# Phase 2 — profile collection (run against the corpus)
# ----------------------------------------------------------------------

echo ""
echo "==> Phase 2: profile collection"
INSTR_BIN="target/x86_64-unknown-linux-gnu/release/examples/profile_corpus"
if [[ ! -x "$INSTR_BIN" ]]; then
    echo "FATAL: expected instrumented binary at $INSTR_BIN" >&2
    exit 3
fi

# Run the instrumented binary multiple times to get a stable profile.
# AOZORA_CORPUS_ROOT is consumed by the example.
for run in 1 2 3; do
    echo "    profile run $run/3..."
    AOZORA_CORPUS_ROOT="$AOZORA_CORPUS_ROOT" "$INSTR_BIN" >/dev/null
done

# ----------------------------------------------------------------------
# Phase 3 — optimised rebuild
# ----------------------------------------------------------------------

echo ""
echo "==> Phase 3: optimised rebuild with collected profile"
cargo pgo optimize build -- -p aozora-cli -p aozora-parser --example profile_corpus

OPT_BIN="target/x86_64-unknown-linux-gnu/release/examples/profile_corpus"
echo ""
echo "==> PGO build complete"
echo "    optimised binary: $OPT_BIN"
ls -lh "$OPT_BIN" 2>/dev/null || true

# ----------------------------------------------------------------------
# Phase 4 (optional) — BOLT post-link
# ----------------------------------------------------------------------

if command -v llvm-bolt >/dev/null 2>&1; then
    echo ""
    echo "==> Phase 4: llvm-bolt post-link optimisation"

    # Collect a perf record of the PGO binary first.
    PERF_DATA="target/release/aozora_pgo.perf.data"
    perf record -e cycles:u -j any,u -o "$PERF_DATA" -- \
        "$OPT_BIN" >/dev/null

    BOLT_OUT="${OPT_BIN}.bolt"
    llvm-bolt "$OPT_BIN" -o "$BOLT_OUT" \
        -data="$PERF_DATA" \
        -reorder-blocks=ext-tsp \
        -reorder-functions=hfsort+ \
        -split-functions \
        -split-all-cold \
        -split-eh \
        -dyno-stats

    echo ""
    echo "==> BOLT-optimised binary: $BOLT_OUT"
    ls -lh "$BOLT_OUT"
else
    echo ""
    echo "==> Phase 4 skipped: llvm-bolt not in PATH"
    echo "    install via: sudo apt install llvm-bolt   (Debian/Ubuntu)"
    echo "    or via the llvm-bolt source build: https://github.com/llvm/llvm-project/tree/main/bolt"
fi

# ----------------------------------------------------------------------
# Reporting
# ----------------------------------------------------------------------

echo ""
echo "==> Done. Compare against the baseline:"
echo "    AOZORA_CORPUS_ROOT='$AOZORA_CORPUS_ROOT' \\"
echo "    hyperfine --warmup 3 \\"
echo "      'target/release/examples/profile_corpus' \\"
echo "      '$OPT_BIN'"
if command -v llvm-bolt >/dev/null 2>&1; then
    echo "    (then add the BOLT binary as a third command:)"
    echo "      '${OPT_BIN}.bolt'"
fi
