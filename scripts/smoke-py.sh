#!/usr/bin/env bash
#
# Host-side Python wheel smoke for aozora-py.
#
# maturin and a Python interpreter are deliberately NOT in the dev image,
# so — like `just smoke-ffi` and `just pgo` — this runs on the HOST, not
# in the container. It provisions a throwaway venv, builds the abi3
# wheel with maturin, installs it, then runs mypy + the pytest suite.
#
# NOT part of `just ci` (the dev image can't run it). The ci.yml
# `python-wheel` job is its CI mirror.
#
# Requirements (host):
#   - cargo            (maturin compiles the extension)
#   - uv  OR  python3 with the stdlib venv + pip modules
#
# Knobs:
#   AOZORA_PY_PYTHON   interpreter version for the venv (default 3.11)
#   AOZORA_PY_VENV     venv path (default target/venv-smoke-py)

set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$(pwd)"

require() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "FATAL: required tool '$1' not found in PATH" >&2
        echo "       installation hint: $2" >&2
        exit 2
    fi
}

require cargo "rustup (https://rustup.rs/)"

CRATE="crates/aozora-py"
VENV="${AOZORA_PY_VENV:-$ROOT/target/venv-smoke-py}"
PYVER="${AOZORA_PY_PYTHON:-3.11}"
TOOLS=('maturin>=1.9,<2.0' 'pytest>=8' 'mypy>=1.11')

note() { echo "[smoke-py] $*"; }

# ── provision an isolated venv with maturin + test tooling ────────────
note "provisioning venv at $VENV"
# Idempotent: create the venv only if it isn't already there (re-runs
# reuse it), then always (re)install the tooling so it's guaranteed
# present and current.
if command -v uv >/dev/null 2>&1; then
    [ -x "$VENV/bin/python" ] || uv venv --python "$PYVER" "$VENV"
    uv pip install --python "$VENV/bin/python" -q "${TOOLS[@]}"
    pip_install() { uv pip install --python "$VENV/bin/python" -q --force-reinstall "$@"; }
else
    require python3 "your OS package manager (needs the venv + pip stdlib modules)"
    [ -x "$VENV/bin/python" ] || python3 -m venv "$VENV"
    "$VENV/bin/python" -m pip install -q --upgrade pip
    "$VENV/bin/python" -m pip install -q "${TOOLS[@]}"
    pip_install() { "$VENV/bin/python" -m pip install -q --force-reinstall "$@"; }
fi

export VIRTUAL_ENV="$VENV"
export PATH="$VENV/bin:$PATH"

# ── build the abi3 wheel, then install it into the venv ───────────────
note "building the wheel (maturin)"
maturin build -F extension-module -m "$CRATE/Cargo.toml" -o "$ROOT/target/wheels"
WHEEL="$(ls -t "$ROOT"/target/wheels/aozora-*.whl | head -1)"
note "installing $(basename "$WHEEL")"
pip_install "$WHEEL"

# ── type-check the wrapper, then run the suite ────────────────────────
note "mypy --strict"
mypy --strict "$CRATE/python"

note "pytest"
python -m pytest "$CRATE/tests" -q

note "OK"
