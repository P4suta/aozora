# Native development and CI entry points.

set shell := ["bash", "-euo", "pipefail", "-c"]
set dotenv-load := false

_FUZZ_TOOLCHAIN := "nightly-2026-07-15"

# --- metadata -----------------------------------------------------------------

# Default: show this help
default:
    @just --list --unsorted

# First-run setup after `mise install --locked`.
setup:
    #!/usr/bin/env bash
    set -euo pipefail
    command -v mise >/dev/null || { echo "❌ mise not found — install it from https://mise.jdx.dev/"; exit 1; }
    export MISE_IGNORED_CONFIG_PATHS="${XDG_CONFIG_HOME:-$HOME/.config}/mise/config.toml"
    mapfile -t tools < <(mise config get -f mise.toml tools \
      | sed '/^\[\[/,$d' \
      | sed -nE 's/^"?([^" =]+)"?[[:space:]]*=.*/\1/p')
    tools+=(rust@1.97.0 rust@nightly-2026-07-15)
    mise install --locked "${tools[@]}"
    stable_toolchain=$(awk -F'"' '/^channel/ { print $2; exit }' rust-toolchain.toml)
    rustup component add --toolchain "$stable_toolchain" clippy rustfmt rust-src llvm-tools-preview
    rustup target add --toolchain "$stable_toolchain" wasm32-unknown-unknown wasm32-wasip1
    rustup component add --toolchain "{{_FUZZ_TOOLCHAIN}}" rust-src miri
    echo "▶ [1/2] Installing git hooks…"
    just hooks
    echo "▶ [2/2] Verifying the toolchain…"
    just test
    echo "✅ Setup complete. Try 'just --list'."

# Diagnose the native development environment without changing it.
doctor:
    #!/usr/bin/env bash
    set -uo pipefail
    fail=0; warn=0
    export MISE_IGNORED_CONFIG_PATHS="${XDG_CONFIG_HOME:-$HOME/.config}/mise/config.toml"
    ok()   { echo "✅ $1"; }
    bad()  { echo "❌ $1"; echo "      ↳ $2"; fail=$((fail + 1)); }
    note() { echo "⚠️  $1"; echo "      ↳ $2"; warn=$((warn + 1)); }

    command -v mise >/dev/null \
      && ok "mise installed" \
      || bad "mise not found" "install mise from https://mise.jdx.dev/"
    mapfile -t tools < <(mise config get -f mise.toml tools \
      | sed '/^\[\[/,$d' \
      | sed -nE 's/^"?([^" =]+)"?[[:space:]]*=.*/\1/p')
    tools+=(rust@1.97.0 rust@nightly-2026-07-15)
    mise install --locked --dry-run "${tools[@]}" >/dev/null 2>&1 \
      && ok "mise.lock matches mise.toml" \
      || bad "mise.lock is stale or missing" "run 'mise lock' and commit the result"
    for tool in cargo rustc just lefthook typos committed actionlint taplo bun node go quicktype wasm-opt; do
      command -v "$tool" >/dev/null \
        && ok "tool: $tool" \
        || bad "tool missing: $tool" "run 'mise install --locked'"
    done
    if [ -f .git/hooks/pre-commit ] && grep -q lefthook .git/hooks/pre-commit 2>/dev/null; then
      ok "git hooks installed (lefthook)"
    elif [ -n "$(git config --get core.hooksPath 2>/dev/null)" ]; then
      ok "git hooks via core.hooksPath ($(git config --get core.hooksPath))"
    else
      bad "git hooks not installed" "run 'just hooks'"
    fi
    { [ "$(git config --get commit.gpgsign 2>/dev/null)" = "true" ] \
        && [ -n "$(git config --get user.signingkey 2>/dev/null)" ]; } \
      && ok "commit signing configured" \
      || note "commit signing not configured" "commits must be signed — see CONTRIBUTING.md"
    [ -f rust-toolchain.toml ] \
      && ok "rust-toolchain.toml present" \
      || bad "rust-toolchain.toml missing" "restore the repository toolchain declaration"
    stable_toolchain=$(awk -F'"' '/^channel/ { print $2; exit }' rust-toolchain.toml)
    installed_components=$(rustup component list --toolchain "$stable_toolchain" --installed 2>/dev/null)
    grep -q '^clippy-' <<<"$installed_components" \
      && ok "Rust component: clippy" \
      || bad "Rust component missing: clippy" "run 'just setup'"
    grep -q '^rustfmt-' <<<"$installed_components" \
      && ok "Rust component: rustfmt" \
      || bad "Rust component missing: rustfmt" "run 'just setup'"
    grep -qx 'rust-src' <<<"$installed_components" \
      && ok "Rust component: rust-src" \
      || bad "Rust component missing: rust-src" "run 'just setup'"
    grep -q '^llvm-tools-' <<<"$installed_components" \
      && ok "Rust component: llvm-tools-preview" \
      || bad "Rust component missing: llvm-tools-preview" "run 'just setup'"
    for target in wasm32-unknown-unknown wasm32-wasip1; do
      rustup target list --toolchain "$stable_toolchain" --installed 2>/dev/null | grep -qx "$target" \
        && ok "Rust target: $target" \
        || bad "Rust target missing: $target" "run 'just setup'"
    done
    rustup component list --toolchain "{{_FUZZ_TOOLCHAIN}}" --installed 2>/dev/null | grep -q '^miri-' \
      && ok "Rust component: miri ({{_FUZZ_TOOLCHAIN}})" \
      || bad "Rust component missing: miri ({{_FUZZ_TOOLCHAIN}})" "run 'just setup'"
    { [ -n "${AOZORA_CORPUS_ROOT:-}" ] && [ -d "${AOZORA_CORPUS_ROOT:-}" ]; } \
      && ok "AOZORA_CORPUS_ROOT set ($AOZORA_CORPUS_ROOT)" \
      || note "AOZORA_CORPUS_ROOT unset (corpus sweeps skip)" "export AOZORA_CORPUS_ROOT=\$HOME/aozora-corpus (optional)"
    avail_kb=$(df -Pk . | awk 'NR==2{print $4}')
    [ "${avail_kb:-0}" -ge 5242880 ] \
      && ok "disk headroom (>= 5 GB free)" \
      || note "less than ~5 GB free here" "Cargo, Bun, and browser artifacts need headroom"
    command -v valgrind >/dev/null \
      && ok "valgrind available (performance gate ready)" \
      || note "valgrind not found" "install it with the operating-system package manager"
    command -v clang >/dev/null \
      && ok "clang available (all-target lint ready)" \
      || note "clang not found" "install clang and libclang development headers with the operating-system package manager"
    if [ -r /proc/sys/kernel/perf_event_paranoid ]; then
      lvl=$(cat /proc/sys/kernel/perf_event_paranoid)
      [ "${lvl:-9}" -le 1 ] \
        && ok "perf_event_paranoid=$lvl (samply profiling ready)" \
        || note "perf_event_paranoid=$lvl (samply needs <= 1)" "echo 1 | sudo tee /proc/sys/kernel/perf_event_paranoid"
    fi

    echo "────"
    if [ "$fail" -gt 0 ]; then
      echo "❌ $fail blocking issue(s), $warn warning(s) — fix the ❌ above (often 'just setup')."
      exit 1
    fi
    echo "✅ no blocking issues ($warn warning(s))."

# --- build --------------------------------------------------------------------

# Package selection shared by every workspace-wide runner below, defined
# once so no two of them can end up selecting a different workspace.
#
# `aozora-bench` is out because it's a bench-only harness whose dep tree
# pulls in `zstd-sys`, `criterion`, `addr2line`, `gimli`, `object`, and
# `ruzstd` — ~100 s of cold-cache compile time no other crate needs. Bench runs go through
# `just bench`, which invokes `cargo bench --workspace` and gets the full
# tree on demand.
_ws := "--workspace --exclude aozora-bench"

# Build all workspace crates.
build:
    cargo build {{_ws}} --all-targets

# Fastest "does it still compile?" gate. `cargo check` skips codegen,
# so it's the inner-loop signal; `just build` stays the --all-targets
# gate that also links the test / example binaries. Mirrors bacon's
# default `check` job and the MSRV CI job's `cargo check --workspace
# --all-targets`, so the "still compiles?" answer is the same surface
# everywhere.
check:
    cargo check {{_ws}} --all-targets

# Build release binaries
build-release:
    cargo build --release {{_ws}}

# Run the aozora CLI with arbitrary args (`just run check FILE`, etc.)
run *ARGS:
    cargo run --package aozora-cli --quiet -- {{ARGS}}

# Run a library example from crates/aozora/examples/ (`just example hello`).
# Each example uses only the `aozora` umbrella surface and prints to stdout.
example NAME *ARGS:
    cargo run -p aozora --example {{NAME}} {{ARGS}}

# --- tests --------------------------------------------------------------------

# Run the full test suite (unit + integration + snapshot).
# `aozora-bench` is excluded — see `build` above for rationale.
test *ARGS:
    cargo nextest run {{_ws}} --all-targets {{ARGS}}

# Run only the tests whose name matches FILTER — the single-test inner
# loop. Uses nextest's filterset DSL: a bare string is a substring
# match, wrap it in slashes for a regex. Extra nextest flags pass
# through after FILTER.
#   just t ruby                # every test whose name contains "ruby"
#   just t '/ruby|bouten/'     # regex
#   just t ruby --no-capture   # forward nextest flags
t FILTER *ARGS:
    cargo nextest run {{_ws}} -E 'test({{FILTER}})' {{ARGS}}

# Run doctests (nextest skips these by design)
test-doc:
    cargo test --workspace --doc

# Doctests for the umbrella crate with its optional features enabled
# (wire / formatter / Pandoc), so feature-gated rustdoc examples are
# verified too. `just test-doc` stays feature-light for speed; run this
# before a release or after touching a feature-gated public example.
test-doc-all:
    cargo test -p aozora --doc --all-features

# Refresh insta snapshot files in place. Used after an intentional
# change to a snapshot-tested surface (rendered HTML, AST `Debug`,
# CLI `--help`). Sets `INSTA_UPDATE=always` so the test runner
# overwrites the on-disk `.snap` files instead of failing on the
# diff. Review `git diff` afterwards before committing — accepting a
# snapshot is reviewing a public surface change.
#
# CLI tool (`cargo insta`) is intentionally not used; the
# `INSTA_UPDATE` env knob is the same surface and stays inside the
# already-vendored `insta` workspace dep.
#
# `--all-features` where `just test` is feature-light: a writer that
# cannot see every snapshot leaves the ones it misses to be hand-edited.
# The feature-gated snapshots (aozora's fmt / pandoc, and the
# in-process LSP folded into aozora-cli) only render under `--all-features`.
snapshot-update:
    env INSTA_UPDATE=always cargo nextest run {{_ws}} --all-features --all-targets

# Phase K3 — byte-identical render gate. Loads aozora-conformance
# fixtures and asserts current parse → render output matches golden
# files. Set UPDATE_GOLDEN=1 to refresh after intentional output
# change.
render-gate:
    cargo test -p aozora-conformance --test render_gate

# Refresh aozora-conformance golden files. Use after intentional
# renderer output changes; commit the resulting fixture diff.
render-gate-update:
    env UPDATE_GOLDEN=1 cargo test -p aozora-conformance --test render_gate

# Phase L1 — regenerate the wire JSON Schema artefacts under
# crates/aozora-conformance/json/. Run after touching any wire struct
# or `aozora::json::SCHEMA_VERSION`; commit the resulting diff so
# `schema-check` (drift gate) stays green.
schema:
    cargo run -p aozora-xtask -q -- schema dump

# Phase L1 / L4 — drift gate: fail if the on-disk wire schemas
# disagree with the live wire structs. Wired into the `drift-gate`
# CI job; run locally before pushing if you touched wire types.
schema-check:
    cargo run -p aozora-xtask -q -- schema check

# Phase L2 — regenerate crates/aozora-wasm/types/aozora_types.d.ts
# from the live enums + wire structs. Commit the diff so
# `types-check` stays green.
types:
    cargo run -p aozora-xtask -q -- types ts

# Phase L2 / L4 — drift gate: fail if the committed
# aozora_types.d.ts disagrees with fresh codegen. Wired into the
# `drift-gate` CI job.
types-check:
    cargo run -p aozora-xtask -q -- types check

# Generate per-language wire types (Go / …) from the committed wire JSON
# Schema via quicktype — one generator, every host-SDK language. Writes
# `crates/aozora-<lang>/…`; commit the diff so `types-langs-check` stays
# green. quicktype + gofmt are provisioned by mise.
types-langs:
    cargo run -p aozora-xtask -q -- types langs

# Drift gate for the per-language wire types. Wired into `drift-gate`.
types-langs-check:
    cargo run -p aozora-xtask -q -- types langs-check

# Regenerate the committed tree-sitter parser
# (crates/tree-sitter-aozora/src/{parser.c,grammar.json,node-types.json})
# from grammar.js via the pinned tree-sitter CLI. Run after an intentional
# grammar.js edit; commit the diff so `grammar-check` (drift gate) stays green.
grammar:
    cargo run -p aozora-xtask -q -- conformance grammar --update

# Drift gate: fail if the committed tree-sitter parser has drifted from a
# fresh `tree-sitter generate` of grammar.js. Wired into `drift-gate`.
grammar-check:
    cargo run -p aozora-xtask -q -- conformance grammar --check

# Drift gate: fail if release-plz.toml's `changelog_include` has drifted
# from the workspace's publishable members (a crate added to the
# workspace but not to that list drops out of the aggregated CHANGELOG),
# or if a manifest breaks the publish-path hygiene rules the root
# Cargo.toml states in prose: path-only internal dev-deps, and no
# registry version on a `publish = false` member.
# Offline — never contacts crates.io. Wired into `drift-gate`.
publish-check:
    cargo run -p aozora-xtask -q -- publish check

# Build and verify the exact local crate archives without publishing them.
artifact-crates *ARGS:
    cargo run -p aozora-xtask -q -- artifacts crates {{ARGS}}

# Rearm preflight: verify every deployed precondition CI-green cannot prove —
# the release-plz / release environment secrets + protection, the server-side
# tag ruleset, a completed-success release-ready for the exact commit, and the
# first-publish registry residue — plus the offline `release check` gate and the
# freeze-latch state. Online (`gh` + registry HTTPS), so it runs with the
# maintainer's `gh` auth. Fails closed on any gap.
# `--offline` runs only the repo-local half; `--first-publish` acknowledges a
# known new crate/project so it does not hard-stop.
rearm-preflight *ARGS:
    cargo run -p aozora-xtask -q -- release preflight {{ARGS}}

# Rearm rehearsal: fire the PyPI / npm publishers' `dry_run` dispatches for the
# qualified commit so their `qualify` jobs run BEFORE the irreversible tag push
# (release.yml / extism resolve the tag in qualify and cannot rehearse pre-tag).
# Dispatches real workflow runs, so it is a deliberate step, on the host.
rearm-rehearse *ARGS:
    cargo run -p aozora-xtask -q -- release rehearse {{ARGS}}

# Drift gate: rust-toolchain.toml's channel (the DEV toolchain) and
# Cargo.toml's rust-version (the PUBLIC CONTRACT) are two authorities
# holding deliberately different numbers (ADR-0034). Fail if a pin follows
# the wrong one, if a maintained doc names a Rust version outside
# contrib/msrv.md, if a README hand-writes the MSRV badge, or if the
# contract drifts within six months of the channel. Wired into `drift-gate`.
msrv-check:
    cargo run -p aozora-xtask -q -- msrv check

# Verify the declared MSRV actually builds — the local mirror of CI's
# `msrv` job. Reads the version from Cargo.toml so it cannot drift from
# the contract it checks. See contrib/msrv.md to re-measure the floor.
msrv-local:
    #!/usr/bin/env bash
    set -euo pipefail
    v=$(awk -F'"' '/^rust-version/ { print $2; exit }' Cargo.toml)
    rustup toolchain install "$v" --profile minimal
    # Two lanes: every feature a crates.io consumer can enable, then
    # aozora-extism on defaults only (publish=false; its dev-only
    # host-smoke feature pulls wasmtime, which declares a higher floor
    # than it needs — a test feature does not get to set the contract).
    cargo "+$v" check --locked --workspace --all-targets --all-features --exclude aozora-extism
    cargo "+$v" check --locked -p aozora-extism --all-targets
    echo "msrv-local: workspace builds on $v"

# Generated-artifact and release-contract drift gate.
drift-gate:
    cargo run -p aozora-xtask -q -- schema check
    cargo run -p aozora-xtask -q -- types check
    cargo run -p aozora-xtask -q -- types langs-check
    cargo run -p aozora-xtask -q -- conformance grammar --check
    cargo run -p aozora-xtask -q -- publish check
    cargo run -p aozora-xtask -q -- release check
    cargo run -p aozora-xtask -q -- msrv check
    cargo run -p aozora-xtask -q -- lint coordinates

# Scaffold a new ADR under docs/adr/ from the template: picks the next
# 4-digit number, slugifies the title, stamps today's date, and writes a
# skeleton. Pure file templating with no toolchain dependencies.
#   just new-adr "Make the lexer streaming"
new-adr TITLE:
    #!/usr/bin/env bash
    set -euo pipefail
    last=$(ls docs/adr/ | grep -oE '^[0-9]{4}' | sort -n | tail -1)
    n=$(printf '%04d' $((10#$last + 1)))
    slug=$(printf '%s' "{{TITLE}}" | tr '[:upper:] ' '[:lower:]-' | tr -cd 'a-z0-9-')
    f="docs/adr/${n}-${slug}.md"
    [[ -e "$f" ]] && { echo "$f already exists" >&2; exit 1; }
    cp docs/adr/0000-template.md "$f"
    sed -i -e "s/^# NNNN. TITLE_HERE/# ${n}. {{TITLE}}/" -e "s/YYYY-MM-DD/$(date +%F)/" "$f"
    echo "Created $f"

# Phase O4 — WPT-style conformance runner.
#   1. `conformance run`     — walks aozora-conformance/fixtures/render/,
#                              compares against the parser's own goldens,
#                              writes a per-case results.json beside the
#                              suite that produced it.
#   2. `conformance vectors` — runs the vendored specification vectors
#                              (spec-vectors/, synced from the sibling
#                              aozora-notation-spec) and holds the parser
#                              to the SPEC's expectations.
#   3. `--implementation tree-sitter` (run + vectors) — replays the
#                              fixtures AND the spec vectors through the
#                              reference grammar and gates on their
#                              S-expression snapshots (expected.tree-sitter.txt
#                              per fixture; spec-vectors/tree-sitter-snapshot.json
#                              for the vectors); any drift exits non-zero
#                              (--update to refresh).
#   4. `works_gate`          — byte-identical golden-HTML gate over a lean
#                              set of REAL vendored 青空文庫 works
#                              (fixtures/works/), catching render drift on
#                              the notation COMBINATIONS the single-family
#                              fixtures miss. Corpus-free (works are
#                              vendored), so it belongs in this always-on
#                              job. UPDATE_GOLDEN=1 refreshes its goldens.
# Any pass exits non-zero on a `must`-tier regression or grammar drift.
conformance:
    bash -c 'set -euo pipefail; cargo run -p aozora-xtask -q -- conformance run && cargo run -p aozora-xtask -q -- conformance vectors && cargo run -p aozora-xtask -q -- conformance run --implementation tree-sitter && cargo run -p aozora-xtask -q -- conformance vectors --implementation tree-sitter && cargo test -p aozora-conformance --test works_gate && cargo run -p aozora-xtask -q -- corpus family-coverage'

sync-spec-vectors:
    #!/usr/bin/env bash
    set -euo pipefail
    spec="${AOZORA_SPEC_REPO:-$PWD/../aozora-notation-spec}"
    [[ -d "$spec" ]] || { echo "spec repository not found: $spec" >&2; exit 1; }
    AOZORA_SPEC_REPO="$spec" cargo run -q --release -p aozora-xtask -- spec-vectors sync

verify-spec-vectors:
    #!/usr/bin/env bash
    set -euo pipefail
    spec="${AOZORA_SPEC_REPO:-$PWD/../aozora-notation-spec}"
    if [[ ! -d "$spec" ]]; then
        echo "spec repository not configured; vendored vectors remain covered by conformance"
        exit 0
    fi
    AOZORA_SPEC_REPO="$spec" cargo run -q --release -p aozora-xtask -- spec-vectors check

# Property-based tests only. Default 128 cases per proptest block
# (AOZORA_PROPTEST_CASES override via aozora-proptest::config). Fast
# enough to live in `just ci` — see `just prop-deep` for a stress run.
prop:
    cargo nextest run --workspace --all-features --test 'property_*' --run-ignored default

# Deep property sweep — 4096 cases per block, used before cutting a
# release to exercise invariants beyond the default CI budget.
prop-deep:
    bash -c 'AOZORA_PROPTEST_CASES=4096 cargo nextest run --workspace --all-features --test "property_*" --run-ignored default'

# Walk every document under `AOZORA_CORPUS_ROOT` and check parse +
# round-trip + source-region tiling + document edit differential
# invariants on the public `aozora::Document` surface.
# Runtime-skips with an informational message if the env var is unset.
#
# Usage:
#   export AOZORA_CORPUS_ROOT=$HOME/aozora-corpus
#   just corpus-sweep
corpus-sweep:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; sweep has nothing to walk."
        echo "Set it to a directory of aozora-format .txt files, e.g.:"
        echo "  export AOZORA_CORPUS_ROOT=\$HOME/aozora-corpus"
        echo "Then re-run 'just corpus-sweep'."
        exit 0
    fi
    if [[ ! -d "$AOZORA_CORPUS_ROOT" ]]; then
        echo "AOZORA_CORPUS_ROOT=$AOZORA_CORPUS_ROOT is not a directory." >&2
        exit 1
    fi
    AOZORA_REQUIRE_CORPUS=1 cargo nextest run --release --package aozora --test corpus_sweep --test corpus_document_edits --no-capture

incremental-speedup-gate:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]] || [[ ! -d "$AOZORA_CORPUS_ROOT" ]]; then
        echo "AOZORA_CORPUS_ROOT must name a readable corpus directory." >&2
        exit 1
    fi
    AOZORA_INCREMENTAL_MIN_SPEEDUP=1.10 \
      AOZORA_INCREMENTAL_MAX_SLOWDOWN=1.50 \
      cargo run -q --release -p aozora-bench --example incremental_speedup

# Strict full-corpus parser gate. The release-ready recipe requires a corpus;
# this developer convenience recipe remains optional when none is configured.
audit-gate:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; audit-gate skipped (no corpus to walk)."
        exit 0
    fi
    if [[ ! -d "$AOZORA_CORPUS_ROOT" ]]; then
        echo "AOZORA_CORPUS_ROOT=$AOZORA_CORPUS_ROOT is not a directory." >&2
        exit 1
    fi
    cargo run -p aozora-xtask -q -- corpus audit-gate --root "$AOZORA_CORPUS_ROOT"

# Verbatim-provenance gate: fail when any corpus document's
# `Tree::to_source_verbatim()` no longer equals a fresh `sanitize()` of
# its decoded source (the I5 invariant). Binary — one byte of drift
# fails; needs no baseline. Uses the same runtime-skip behavior as
# `audit-gate` when AOZORA_CORPUS_ROOT is unset.
#
# Usage:
#   export AOZORA_CORPUS_ROOT=$HOME/aozora-corpus
#   just verbatim-gate
verbatim-gate:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; verbatim-gate skipped (no corpus to walk)."
        exit 0
    fi
    if [[ ! -d "$AOZORA_CORPUS_ROOT" ]]; then
        echo "AOZORA_CORPUS_ROOT=$AOZORA_CORPUS_ROOT is not a directory." >&2
        exit 1
    fi
    cargo run --release -p aozora-xtask -q -- corpus verbatim --root "$AOZORA_CORPUS_ROOT"

# Strict unexplained visible-notation gate.
render-leak-gate:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; render-leak-gate skipped (no corpus to walk)."
        exit 0
    fi
    if [[ ! -d "$AOZORA_CORPUS_ROOT" ]]; then
        echo "AOZORA_CORPUS_ROOT=$AOZORA_CORPUS_ROOT is not a directory." >&2
        exit 1
    fi
    cargo run --release -p aozora-xtask -q -- corpus render-leak-gate --root "$AOZORA_CORPUS_ROOT"

# Strict structural render gate.
render-correctness-gate:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; render-correctness-gate skipped (no corpus to walk)."
        exit 0
    fi
    if [[ ! -d "$AOZORA_CORPUS_ROOT" ]]; then
        echo "AOZORA_CORPUS_ROOT=$AOZORA_CORPUS_ROOT is not a directory." >&2
        exit 1
    fi
    cargo run --release -p aozora-xtask -q -- corpus render-correctness-gate --root "$AOZORA_CORPUS_ROOT"

# Select a stratified, family-diverse set of real works to extend the golden
# `fixtures/works/` set (#414). Deterministic greedy family set-cover under a
# source-byte budget; writes `fixtures/works-selection.toml`.
works-select:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; cannot select works." >&2
        exit 1
    fi
    cargo run -p aozora-xtask -q -- corpus select-works --root "$AOZORA_CORPUS_ROOT"

# Vendor the works named in `works-selection.toml` into `fixtures/works/` and
# seed their golden HTML. Run after `works-select` (and any manual slug edits).
works-vendor:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; cannot vendor works." >&2
        exit 1
    fi
    cargo run -p aozora-xtask -q -- corpus vendor-works --root "$AOZORA_CORPUS_ROOT"
    UPDATE_GOLDEN=1 cargo test -p aozora-conformance --test works_gate

# Public-document allocation-pressure ratchet. Measures parse allocation over
# the corpus plus fixed edit, snapshot, large-document, and render workloads.
# Same local runtime-skip as `audit-gate`.
#
# Usage:
#   export AOZORA_CORPUS_ROOT=$HOME/aozora-corpus
#   just alloc-gate
alloc-gate: baseline-ratchet
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; alloc-gate skipped (no corpus to walk)."
        exit 0
    fi
    if [[ ! -d "$AOZORA_CORPUS_ROOT" ]]; then
        echo "AOZORA_CORPUS_ROOT=$AOZORA_CORPUS_ROOT is not a directory." >&2
        exit 1
    fi
    cargo run --release -p aozora-bench --example alloc_gate -- --root "$AOZORA_CORPUS_ROOT" --baseline corpus/alloc-baseline.json

# Re-capture the allocation baseline. The baseline ratchet rejects increases.
alloc-gate-update:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; cannot capture a baseline." >&2
        exit 1
    fi
    cargo run --release -p aozora-bench --example alloc_gate -- --root "$AOZORA_CORPUS_ROOT" --baseline corpus/alloc-baseline.json --update

# Tier-2 wall-clock validation (#237 P0.2-real): owned-vs-borrowed lex MB/s per
# size band, a same-machine self-baselining ratio. NOT a gate — run at the
# P0.2-real landing commit and record the band ratios in the PR description.
throughput:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; throughput skipped (no corpus to walk)."
        exit 0
    fi
    cargo run --release -p aozora-bench --example throughput

# Instruction-count perf gate (G6). Runs fixed micro-benchmarks under
# Valgrind's Callgrind, which counts
# CPU *instructions* — stable within a fixed native measurement epoch, unlike
# wall-clock (too noisy on shared runners to gate on; see `throughput`).
#
# Every run compares against the committed baseline and exits non-zero on an
# instruction regression on any case. Corpus-free — the runner embeds a
# few vendored 青空文庫 works plus a synthetic annotation-dense buffer — so
# it needs no AOZORA_CORPUS_ROOT. Requires Valgrind on Linux.
#
# Runs in the local pre-push gate and required CI.
#
baseline-ratchet:
    cargo run -p aozora-xtask -q -- ratchet

perf-gate: baseline-ratchet
    cargo run -p aozora-xtask -q -- perf check

artifact-size-gate: baseline-ratchet extism-build
    env CARGO_PROFILE_RELEASE_OPT_LEVEL=z wasm-pack build --target web --release crates/aozora-wasm --locked
    sh -c 'set -eu; wasm=crates/aozora-wasm/pkg/aozora_wasm_bg.wasm; \
        before=$(wc -c < "$wasm"); \
        wasm-opt -Oz --strip-debug --strip-dwarf --vacuum \
            --enable-bulk-memory --enable-mutable-globals \
            --enable-nontrapping-float-to-int "$wasm" -o "$wasm"; \
        after=$(wc -c < "$wasm"); test "$after" -lt "$before"'
    bash -euc 'cargo build --locked --profile dist -p aozora-cli; \
        mkdir -p target/release-ready-build; \
        cp target/dist/aozora target/release-ready-build/'
    cargo run -p aozora-xtask -q -- artifacts size-check

# --- fuzzing -----------------------------------------------------------------
#
# cargo-fuzz harnesses live under `crates/aozora/fuzz/<CRATE>/` (CRATE ∈
# {pipeline, render, encoding}) as nightly-only sub-crates outside the main
# workspace (so the workspace build doesn't pull libfuzzer-sys). They target the
# collapsed `aozora` surface through the public document and render APIs,
# `aozora::encoding::*`. Targets currently registered:
#
#   pipeline / lex
#   pipeline / ffi_no_abort
#   render   / render_html
#   render   / serialize_round_trip
#   render   / catalogue_normalization
#   encoding / decode_sjis
#
# Workflow:
#   1. `just fuzz-quick CRATE TARGET`    (60 s) — inner-loop smoke
#   2. `just fuzz-deep  CRATE TARGET`    (5 min) — release pre-flight
#   3. `just fuzz-marathon CRATE TARGET` (15 min) — strongest soak
#   4. On crash, `just fuzz-triage CRATE TARGET` prints just the panic
#      block (panic line + diagnostic context) for every artifact under
#      crates/aozora/fuzz/<CRATE>/artifacts/<target>/. No manual repro loop.
#   5. `just fuzz-promote CRATE TARGET ARTIFACT` lifts an artifact into
#      crates/aozora/tests/fuzz_regressions/<CRATE>/<target>/ so the
#      regression integration test replays it on every `just test` run — no
#      nightly required for the regression case.
#   6. `just fuzz-status` is the at-a-glance count of pending crashes
#      vs pinned regressions per target.

_fuzz_target := "x86_64-unknown-linux-gnu"

# Run an arbitrary fuzz target with arbitrary args (escape hatch — the caller
# supplies TARGET, the `--target` triple (see `_fuzz_target` above), and any
# libFuzzer args). The gated recipes below inject `--target` for you.
fuzz CRATE *ARGS:
    bash -c 'cd crates/aozora/fuzz/{{CRATE}} && cargo +{{_FUZZ_TOOLCHAIN}} fuzz run --fuzz-dir . {{ARGS}}'

# 60-second smoke fuzz — fits inside a development inner loop.
fuzz-quick CRATE TARGET:
    bash -c 'cd crates/aozora/fuzz/{{CRATE}} && cargo +{{_FUZZ_TOOLCHAIN}} fuzz run --fuzz-dir . --target {{_fuzz_target}} {{TARGET}} -- -max_total_time=60'

# 5-minute deep fuzz — the gate to clear before tagging a release.
fuzz-deep CRATE TARGET:
    bash -c 'cd crates/aozora/fuzz/{{CRATE}} && cargo +{{_FUZZ_TOOLCHAIN}} fuzz run --fuzz-dir . --target {{_fuzz_target}} {{TARGET}} -- -max_total_time=300'

# 15-minute marathon fuzz — the strongest single-target soak we run by
# hand. Reach for this after a clean fuzz-deep cycle when you want to
# push the corpus another order of magnitude.
fuzz-marathon CRATE TARGET:
    bash -c 'cd crates/aozora/fuzz/{{CRATE}} && cargo +{{_FUZZ_TOOLCHAIN}} fuzz run --fuzz-dir . --target {{_fuzz_target}} {{TARGET}} -- -max_total_time=900'

# Run every registered fuzz target in turn for 60 s each.
fuzz-all-quick:
    just fuzz-quick pipeline lex
    just fuzz-quick pipeline ffi_no_abort
    just fuzz-quick render render_html
    just fuzz-quick render serialize_round_trip
    just fuzz-quick render catalogue_normalization
    just fuzz-quick encoding decode_sjis

# Run every registered fuzz target in turn for 5 min each — the
# release pre-flight gate.
fuzz-all-deep:
    just fuzz-deep pipeline lex
    just fuzz-deep pipeline ffi_no_abort
    just fuzz-deep render render_html
    just fuzz-deep render serialize_round_trip
    just fuzz-deep render catalogue_normalization
    just fuzz-deep encoding decode_sjis

# Reproduce every artifact under crates/<crate>/fuzz/artifacts/<target>/
# and print the panic block (panicked-at line + diagnostic context the
# fuzz target embeds in the panic message). Exit status is the count of
# artifacts that still crash so it can drive a CI gate.
fuzz-triage CRATE TARGET:
    #!/usr/bin/env bash
    set -euo pipefail
    crate="{{CRATE}}"
    target="{{TARGET}}"
    art_dir="crates/aozora/fuzz/${crate}/artifacts/${target}"
    if [[ ! -d "$art_dir" ]]; then
        echo "fuzz-triage: no artifacts for ${crate} / ${target}"
        exit 0
    fi
    failed=0
    for art in $(find "$art_dir" -type f \( -name 'crash-*' -o -name 'leak-*' -o -name 'oom-*' \) | sort); do
        # `cargo fuzz run` resolves relative paths against the fuzz
        # crate's own directory (we cd into crates/<crate>/fuzz before
        # invoking it), so strip the prefix accordingly.
        rel="${art#crates/aozora/fuzz/${crate}/}"
        echo "==> ${art}"
        out=$(bash -c "cd crates/aozora/fuzz/${crate} && cargo +{{_FUZZ_TOOLCHAIN}} fuzz run --fuzz-dir . --target {{_fuzz_target}} ${target} ${rel} 2>&1" || true)
        # Slice out the panic block: from `thread … panicked at`
        # through the line just before the stack trace begins. That's
        # exactly where the fuzz target's panic message prints its
        # diagnostic context. Falls back to the tail of the output so
        # we never go silent.
        panic_block=$(awk '
            /^thread .* panicked at/ { capturing = 1 }
            capturing {
                if (/^stack backtrace:/ || /^=================/) exit
                print
            }
        ' <<<"$out")
        if [[ -n "$panic_block" ]]; then
            printf "%s\n" "$panic_block"
        else
            tail -5 <<<"$out"
        fi
        if grep -q "exit status: 77" <<<"$out"; then
            failed=$((failed + 1))
        fi
        echo
    done
    if (( failed > 0 )); then
        echo "fuzz-triage: ${failed} artifact(s) still crash" >&2
        exit "${failed}"
    fi
    echo "fuzz-triage: every artifact replays cleanly"

# Lift a fuzz artifact into the permanent regression set so the
# `tests/fuzz_regressions.rs` integration test pins it on every
# `just test` run.
fuzz-promote CRATE TARGET ARTIFACT:
    #!/usr/bin/env bash
    set -euo pipefail
    src="crates/aozora/fuzz/{{CRATE}}/artifacts/{{TARGET}}/{{ARTIFACT}}"
    dst_dir="crates/aozora/tests/fuzz_regressions/{{CRATE}}/{{TARGET}}"
    if [[ ! -f "$src" ]]; then
        echo "fuzz-promote: artifact not found: $src" >&2
        exit 1
    fi
    bash -c "mkdir -p '$dst_dir' && mv '$src' '$dst_dir/{{ARTIFACT}}'"
    echo "promoted ${src} -> ${dst_dir}/{{ARTIFACT}}"

# At-a-glance health: per-target pending crashes vs pinned regressions.
# Nothing here invokes nightly so it stays cheap and shell-friendly.
fuzz-status:
    #!/usr/bin/env bash
    set -euo pipefail
    targets=(
        "pipeline lex"
        "pipeline ffi_no_abort"
        "render render_html"
        "render serialize_round_trip"
        "render catalogue_normalization"
        "encoding decode_sjis"
    )
    printf "%-22s  %-22s  %-10s  %-12s\n" crate target pending_crashes pinned_regressions
    printf "%-22s  %-22s  %-10s  %-12s\n" ---------------------- ---------------------- ---------- ------------
    for entry in "${targets[@]}"; do
        crate="${entry% *}"
        target="${entry#* }"
        crashes=0
        regressions=0
        art_dir="crates/aozora/fuzz/${crate}/artifacts/${target}"
        reg_dir="crates/aozora/tests/fuzz_regressions/${crate}/${target}"
        if [[ -d "$art_dir" ]]; then
            crashes=$(find "$art_dir" -maxdepth 1 -type f \( -name 'crash-*' -o -name 'leak-*' -o -name 'oom-*' \) 2>/dev/null | wc -l | tr -d ' ')
        fi
        if [[ -d "$reg_dir" ]]; then
            regressions=$(find "$reg_dir" -maxdepth 1 -type f ! -name '*.txt' ! -name '*.md' 2>/dev/null | wc -l | tr -d ' ')
        fi
        printf "%-22s  %-22s  %-10s  %-12s\n" "$crate" "$target" "$crashes" "$regressions"
    done

# Benchmarks (criterion)
bench *ARGS:
    cargo bench --workspace {{ARGS}}

# Save the current bench output as a named baseline that
# `bench-compare` can diff against later. Use before a refactor to
# pin "as-of" perf, then run `bench-compare <name>` post-change to
# get criterion's statistical comparison (mean change ± p-value)
# against the same baseline.
#
# Manual / release-cut workflow only — `just ci` does NOT call this.
# Bench drift gating in CI is intentionally avoided: shared GHA
# runners have too much per-job noise for a 5%-threshold to be
# trustworthy without a self-hosted runner. Local runs on the
# author's machine give a stable signal at the cost of being
# discretionary.
bench-baseline NAME="main":
    cargo bench --workspace -- --save-baseline {{NAME}}

# Re-run benches and compare against an earlier saved baseline.
# Criterion prints mean / stddev / p-value per bench; a regression
# > 5% with `change.p_value < 0.05` is a meaningful signal worth
# investigating before cutting a release.
bench-compare NAME="main":
    cargo bench --workspace -- --baseline {{NAME}}

# Heap-allocation profile (dhat) of a 2 MiB synthetic parse + render:
# total allocations + peak bytes, plus dhat-heap.json for dh_view. dhat
# needs no perf_event_open.
dhat:
    cargo run --release -p aozora-bench --example dhat_parse

# Corpus-free small-doc parse+render latency percentiles (p50/p90/p99/max)
# over a synthetic buffer. The deep per-phase, corpus-driven view is the
# `latency_histogram` example.
latency:
    cargo run --release -p aozora-bench --example latency_synthetic

# --- coverage -----------------------------------------------------------------

# Coverage gate. Fails when region coverage drops below `_COV_FLOOR`.
#
# Tool / metric rationale:
# - `cargo-llvm-cov` 0.8.5 supports `--fail-under-regions` and
#   `--fail-under-lines` / `--fail-under-functions`, but not
#   `--fail-under-branches` (the flag simply does not exist in this
#   version). Regions are a strictly finer-grained unit than branches:
#   every conditional in Rust produces separate regions for each
#   outcome, plus finer internal splits. Passing a given region
#   threshold therefore implies at least that branch threshold —
#   region coverage is an honest, stable-toolchain proxy for C1.
# - `--branch` emits branch-level counts only on nightly rustc. We stay
#   on stable for the CI gate (see `rust-toolchain.toml`) and use
#   `coverage-branch` below for informational branch reporting.
#
# Scope excludes:
# - `target/` — build artefacts.
# - `**/main.rs` — CLI binary entrypoints (`aozora-cli`). Thin shells
#   over their crate libraries.
#
# `_COV_FLOOR` is the enforced minimum, not the goal. The workspace
# policy targets 100% on production code; the floor ratchets upward
# in follow-up commits that close specific gaps. The 2026-06-19
# coverage push (render / pandoc / cli / trace / xtask / syntax test
# suites) lifted the workspace to 86.20%; the floor moves to 84 with a
# ~2-point margin so proptest-driven region variance and borderline
# refactors don't trip the gate spuriously — push it up by hand
# whenever a coverage-closing PR lands.
_COV_FLOOR := "84"
_COV_IGNORE := "(target/|/main\\.rs$)"

coverage:
    # Purge the instrumented build dir first. `cargo` never garbage-
    # collects the old hash-suffixed rlib / test binaries of a *renamed
    # or moved* source file, and `cargo llvm-cov clean --workspace`
    # leaves the stale test binaries behind — so on the persistent local
    # dev volume `llvm-cov` would aggregate the pre-rename coverage map as
    # a phantom "ghost" file, inflating the region denominator and
    # deflating the percentage. (CI's volume is ephemeral, so it never
    # sees this — but the local pre-push gate is the authoritative one
    # and must measure correctly, with no skip.) sccache keeps the
    # rebuild cheap (~45 s warm) since the compile cache survives the rm.
    sh -c 'rm -rf "${CARGO_TARGET_DIR:-target}/llvm-cov-target"'
    cargo llvm-cov nextest \
        {{_ws}} \
        --ignore-filename-regex '{{_COV_IGNORE}}' \
        --fail-under-regions {{_COV_FLOOR}}

# Run the in-process LSP's integration suites (smoke / guardian /
# concurrent_lsp / concurrency_regressions / shuttle / fuzz_regressions /
# rope_src_parity / incremental_rope_e2e / incremental_parse_cache_corpus),
# re-homed as `#[cfg(test)]` modules under `src/lsp/` when the LSP itself
# folded into `aozora-cli` (WS-2, #523 withdrawn). Scoped to `-p aozora-cli
# --all-features` on purpose: a blanket `--workspace --all-features` would also
# flip on feature flags whose tests need external setup — e.g. aozora-extism's
# `host-smoke`, which loads a pre-built wasm artifact produced by
# `extism-build` / `smoke-extism` and fails on a clean runner.
test-internals:
    cargo nextest run -p aozora-cli --all-features

# HTML coverage report for local inspection. No threshold — intended
# for opening `coverage/html/index.html` in a browser.
coverage-html:
    cargo llvm-cov nextest \
        {{_ws}} \
        --ignore-filename-regex '{{_COV_IGNORE}}' \
        --html --output-dir coverage/html

# Branch-level coverage report (requires nightly for `--branch` support).
# Informational only — no threshold. Use to surface uncovered conditionals
# when working a specific file toward C1 100%.
coverage-branch:
    cargo +{{_FUZZ_TOOLCHAIN}} llvm-cov nextest \
        --branch \
        {{_ws}} \
        --ignore-filename-regex '{{_COV_IGNORE}}'

# --- mutation testing --------------------------------------------------------

# Assertion-strength gate (cargo-mutants). Mutates the source and checks
# the suite CATCHES each change — the complement region coverage can't
# give: coverage proves a line ran, mutation proves a wrong result would
# fail a test (ADR-0031). A sweep succeeds only when no viable survivor or
# timeout remains; `#[mutants::skip]` needs an equivalence or reachability
# reason.
#
#   just mutants -p aozora-ffi                          # one crate (fast)
#   just mutants --in-diff <(git diff origin/main)      # only changed lines
#
# Runs in a dedicated target directory with incremental compilation enabled.
# Config: repo-root `mutants.toml` via --config (`.gitignore` hides the
# tool's default `.cargo/` location).
#
# `set -f` disables this shell's own pathname expansion so a forwarded
# `--exclude 'src/foo/**'` glob reaches cargo-mutants verbatim instead of
# being expanded against the host tree first (cargo-mutants does its own
# glob matching). Fixed args carry no glob metacharacters, so it is a
# no-op for every other invocation.
mutants *ARGS:
    set -f; CARGO_TARGET_DIR=target/mutants CARGO_INCREMENTAL=1 RUSTC_WRAPPER= \
      cargo mutants --config mutants.toml -j 1 {{ARGS}}

# --- lint / static analysis ---------------------------------------------------

# Run all lints.
lint: fmt-check clippy typos doc

# Build rustdoc with `-D warnings`. Mirrors the `docs` workflow's
# `Build rustdoc` step so a doc-link or rustdoc-lint regression fails
# locally before it reaches the Pages deploy. Stays scoped to the
# workspace lint config (`broken_intra_doc_links = "deny"` in
# `[workspace.lints.rustdoc]` plus the `RUSTDOCFLAGS` env to lift the
# remaining warn-level lints to errors).
# `--jobs 1` serialises rustdoc. `cargo doc --workspace` otherwise spawns
# one rustdoc per crate in parallel, and those processes race on rustdoc's
# *shared* output infrastructure under `target/doc` (`static.files/`,
# `trait.impl/`, `type.impl/`, the cross-crate search index). The race
# surfaces intermittently as `No such file or directory (os error 2)` /
# `failed to create or modify file: I/O error` while documenting the
# umbrella `aozora` crate's re-export pages (e.g. `aozora/pipeline/…`) — it
# bit the `doc` CI job once and needed a rerun. Serialising removes the
# concurrency that is the race's precondition, making the gate
# deterministic; doc generation is fast enough that the throughput cost is
# negligible against not having to rerun a flaky required check.
doc:
    env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features --jobs 1

# Format check (no-write): Rust (rustfmt) + TOML (taplo, taplo.toml policy)
fmt-check:
    cargo fmt --all -- --check
    taplo fmt --check

# Auto-format (writes): Rust (rustfmt) + TOML (taplo, taplo.toml policy)
fmt:
    cargo fmt --all
    taplo fmt

# License and copyright coverage for every repository file.
reuse:
    reuse lint

# Clippy — lint groups (pedantic/nursery/cargo) and carve-outs are owned
# entirely by `[workspace.lints]` in Cargo.toml. Passing `-W clippy::<group>`
# here would re-enable the whole group at CLI priority and silently undo
# per-lint allow carve-outs (e.g. `redundant_pub_crate`). Keep the CLI
# surface to `-D warnings` only.
# `--lib --bins --tests` instead of `--all-targets` skips the
# example / bench targets. `aozora-bench` (and any micro-benches folded
# into `aozora`) declare `[[bench]]` entries that pull
# `criterion`'s entire dep tree (zstd / object / addr2line / gimli)
# into the clippy build for no real lint signal — clippy on a bench
# harness almost never fires anything that wouldn't fire on the lib
# it benches. Bench breakage gets caught the moment you actually
# run `just bench`, where it should.
clippy:
    cargo clippy {{_ws}} --lib --bins --all-features -- -D warnings
    cargo clippy {{_ws}} --tests --all-features -- \
      -D warnings -A clippy::unwrap_used -A clippy::expect_used \
      -A clippy::panic -A clippy::print_stdout -A clippy::print_stderr

# Strict variant: full `--all-targets` (lib + bins + tests + examples
# + benches), and the bench crate is no longer excluded. This is the
# AUTHORITATIVE lint surface — it matches GitHub's `rust` job and is run
# by the pre-push gate, so a
# doc_markdown / missing_docs slip in a bench or example target is
# caught locally before the push, not by CI. The per-commit hook stays
# on the lighter `clippy` (no bench dep tree) for fast feedback.
clippy-strict:
    cargo clippy --workspace --lib --bins --all-features -- -D warnings
    cargo clippy --workspace --examples --tests --benches --all-features -- \
      -D warnings -A clippy::unwrap_used -A clippy::expect_used \
      -A clippy::panic -A clippy::print_stdout -A clippy::print_stderr

# wasm32-target clippy for the wasm-bindgen / Extism plugin crates.
# The host `clippy` recipe builds for the NATIVE target, so it never
# compiles the `#[cfg(target_arch = "wasm32")]` binding modules — their
# lint debt would otherwise stay invisible to every host gate. This
# recipe closes that gap by linting the two wasm32 crates on their real
# target. mise installs wasm32-unknown-unknown (see `extism-build`).
clippy-wasm:
    cargo clippy --target wasm32-unknown-unknown -p aozora-wasm -p aozora-extism -- -D warnings

# Thorough local lint — the --all-targets clippy surface (bench /
# example targets included) plus fmt / typos / doc. Run
# before cutting a release or after touching a bench / example target.
# The per-commit hook runs only the lighter `clippy`.
lint-full: fmt-check clippy-strict clippy-wasm typos doc

# Typo check
typos:
    typos

# Dependency linting (licenses, advisories, bans)
deny:
    cargo deny check

# RustSec advisory scan
audit:
    cargo audit --ignore RUSTSEC-2026-0222

# Unused-dependency scan. cargo-shear is stable (no nightly), fast, and
# also flags unlinked source files; it replaces the former nightly
# cargo-udeps gate. Covers the whole workspace — `aozora-bench` included
# — in a single pass, so no separate bench run is needed. Intentional
# optional deps are carved out via `[package.metadata.cargo-shear]`.
shear:
    cargo shear

# Semver break detection against the crates.io baseline. cargo-semver-checks
# hard-aborts the whole run on the first publishable crate with no registry
# baseline, so exclude the crates that have never been published — until their
# first crates.io release, at which point they drop off this list. (Bin-only
# and `publish = false` members are skipped automatically; a real break makes
# the run exit non-zero, which is expected on a breaking release.) After the
# package collapse the only checked crate is `aozora`;
# `aozora-cli` is bin-only (auto-skipped) and `tree-sitter-aozora` is a first
# publish with no baseline yet.
#
# This is also the ONLY guard for the `#[non_exhaustive]` contract on the
# public `DiagnosticInfo` / `CatalogueMatch` / `CatalogueEntry` types: that
# attribute is inert within the defining crate, so no in-crate test can catch
# a regression (a removed field/variant, or the attribute itself dropped).
# cargo-semver-checks does, cross-crate — keep these types in scope here.
semver *ARGS:
    cargo semver-checks check-release --workspace \
        --exclude tree-sitter-aozora {{ARGS}}

# --- dependency follow-up ----------------------------------------------------
# Dependabot proposes repository updates. The local `deps-check` adds the full
# dependency-health gate (outdated + audit + deny), and its systemd timer
# surfaces new advisories even on quiet branches.

# `target/.deps-check.timestamp` is the last-success marker that
# `deps-status` reads. Written under `target/` and intentionally ephemeral —
# `cargo clean`
# wipes it, which prompts a fresh `deps-check`.
_deps_marker := "target/.deps-check.timestamp"

# Show out-of-date workspace deps (root deps only — transitive bumps
# are noise unless they break something). Exit 0 even when something
# is outdated; this recipe is for inspection, not for gating.
outdated:
    cargo outdated --workspace --root-deps-only --depth 2 --exit-code 0

# Bump every workspace dep to the latest semver-compatible version
# and re-resolve `Cargo.lock`. Safe to run anytime; rejects
# major-version bumps (use `upgrade-incompat` for those, opt-in,
# review-required).
upgrade:
    cargo upgrade --workspace --pinned --recursive
    cargo update --workspace
    @echo "Lockfile updated. Run 'just ci' before committing to verify."

# Bump every workspace dep including major-version (incompatible)
# bumps. Always review the Cargo.toml diff afterwards — major bumps
# are API breaks by definition, and the build / test gate is the
# only thing that catches breakage.
upgrade-incompat:
    cargo upgrade --workspace --incompatible allow --recursive
    cargo update --workspace
    @echo "Lockfile updated WITH incompatible bumps. Review 'git diff Cargo.toml' before committing."

# Full dependency-health gate: outdated + audit + deny. Marks
# `target/.deps-check.timestamp` on success so `deps-status` can
# report freshness. Designed to be runnable from a systemd user timer
# (no TTY requirement, no destructive side effects).
deps-check:
    @mkdir -p target
    @echo "[deps-check] $(date -u +%FT%TZ) — outdated, audit, deny"
    just outdated
    just audit
    just deny
    @date -u +%FT%TZ > {{_deps_marker}}
    @echo "[deps-check] OK — marker written to {{_deps_marker}}"

# Install the systemd user timer that runs `just deps-check` weekly.
# Pure-Rust implementation in `crates/aozora-xtask/src/deps.rs` —
# bound to the *current* repo checkout (the unit bakes in
# `WorkingDirectory=$REPO`). Idempotent. `systemctl --user` binds the timer
# to the current user's session.
deps-timer-install:
    cargo run --release -p aozora-xtask -- deps install-timer

# Show the timer's current state + most recent journal entries.
deps-timer-status:
    cargo run --release -p aozora-xtask -- deps status

# Remove the timer. Preserves the rolling log file under
# `$XDG_STATE_HOME/aozora/deps-check.log`.
deps-timer-uninstall:
    cargo run --release -p aozora-xtask -- deps uninstall-timer

# Print the freshness of the last `deps-check`. Exit non-zero if it
# has been more than 7 days, so shells / CI / hooks can wire it as
# "deps stale" detection without parsing dates.
deps-status:
    @if [ ! -f {{_deps_marker}} ]; then \
        echo "[deps-status] never run; run 'just deps-check'"; \
        exit 1; \
    fi
    @ts="$(cat {{_deps_marker}})"; \
    age_secs=$(( $(date -u +%s) - $(date -u -d "$ts" +%s) )); \
    age_days=$(( age_secs / 86400 )); \
    if [ "$age_days" -gt 7 ]; then \
        echo "[deps-status] last check $age_days days ago ($ts) — STALE; run 'just deps-check'"; \
        exit 1; \
    else \
        echo "[deps-status] last check $age_days days ago ($ts) — fresh"; \
    fi

# --- release optimisation ----------------------------------------------------

# PGO release build from a corpus checkout visible inside the workspace.
pgo:
    env AOZORA_CORPUS_ROOT='{{ env_var("AOZORA_CORPUS_ROOT") }}' bash scripts/pgo-build.sh

# C ABI smoke test — builds aozora-ffi as cdylib, compiles the C
# harness against it, runs end-to-end.
smoke-ffi:
    bash crates/aozora-ffi/tests/c_smoke/run.sh

# Build the single portable `aozora.wasm` Extism plugin (the polyglot
# transport hub) and copy it to crates/aozora-extism/dist/. Every
# language with an Extism host SDK loads this ONE artifact — there is no
# per-(OS × arch) native build matrix the way the aozora-ffi C ABI needs.
# Binaryen's `wasm-opt` is pinned by mise.
extism-build:
    env CARGO_PROFILE_RELEASE_OPT_LEVEL=z cargo build --release --target wasm32-unknown-unknown -p aozora-extism
    sh -c 'set -eu; command -v wasm-opt >/dev/null; \
        mkdir -p crates/aozora-extism/dist \
        && cp "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/aozora_extism.wasm" crates/aozora-extism/dist/aozora.wasm \
        && before=$(wc -c < crates/aozora-extism/dist/aozora.wasm) \
        && wasm-opt -Oz --strip-debug --strip-dwarf --vacuum \
            --enable-bulk-memory --enable-mutable-globals \
            --enable-nontrapping-float-to-int \
            crates/aozora-extism/dist/aozora.wasm -o crates/aozora-extism/dist/aozora.wasm \
        && after=$(wc -c < crates/aozora-extism/dist/aozora.wasm) \
        && test "$after" -lt "$before"'

# End-to-end cross-language ABI check (the Extism analogue of smoke-ffi):
# build the plugin, then load the built aozora.wasm through the Extism
# (Rust) host SDK and assert every export is byte-identical to calling
# aozora::json in-process. The `host-smoke` feature pulls wasmtime, so it
# is opt-in and never burdens `just test` / `just ci`.
smoke-extism: extism-build
    cargo test -p aozora-extism --features host-smoke --test host_smoke -- --nocapture

# End-to-end Go host SDK check (the Go analogue of smoke-ffi / smoke-extism):
# build the plugin, embed it in the Go package, and run `go test`, which
# loads aozora.wasm through the pure-Go wazero Extism runtime and decodes
# every wire envelope into the quicktype-generated Go structs.
smoke-go: extism-build
    bash -c 'set -euo pipefail; \
        cp crates/aozora-extism/dist/aozora.wasm crates/aozora-go/aozora.wasm; \
        cd crates/aozora-go; \
        unformatted=$(gofmt -l .); \
        if [ -n "$unformatted" ]; then echo "gofmt needs: $unformatted"; exit 1; fi; \
        go vet ./...; \
        go test ./...'

# Python wheel smoke. Provisions a throwaway venv, builds and installs the
# abi3 wheel, then runs mypy and pytest, including fixture parity.
smoke-py:
    env AOZORA_PY_VENV=target/venv-smoke-py bash scripts/smoke-py.sh

# Cross-surface parity gate — wasm (Node) channel. Builds the wasm-pack
# `--target nodejs` package and walks every render fixture through it,
# asserting each surface (html / serialize / diagnostics / nodes / pairs /
# nested pairs) is byte-identical to the committed golden — the same
# golden the in-process `render_gate` pins. The `--target web` pkg the
# playground consumes is a separate out-dir, so this leaves it untouched.
# Wired into the fixed web CI suite. The sibling
# CLI / FFI / Python / Go walkers cover the other channels.
parity-wasm:
    bash -euc 'CARGO_PROFILE_RELEASE_OPT_LEVEL=z wasm-pack build --target nodejs --release crates/aozora-wasm --out-dir pkg-nodejs \
        && node crates/aozora-wasm/tests/js/parity.mjs crates/aozora-wasm/pkg-nodejs'

# --- fixed CI suites ----------------------------------------------------------

ci-profile *ARGS:
    cargo run -q --release -p aozora-xtask -- ci profile {{ARGS}}

actionlint:
    actionlint -no-color -shellcheck=shellcheck -pyflakes=

zizmor:
    zizmor --offline --no-progress --min-severity high --min-confidence high .

test-aarch64:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ "$(uname -m)" == "aarch64" ]] || { echo "test-aarch64 requires a native arm64 host" >&2; exit 2; }
    cargo test --locked -p aozora

test-wasm:
    cargo build --locked --target wasm32-wasip1 -p aozora

ci-rust:
    just lint-full
    just build
    just test
    just test-doc
    just test-doc-all
    just test-internals
    just prop
    just coverage
    just conformance
    just drift-gate

ci-web:
    just clippy-wasm
    just parity-wasm
    just playground-ci
    just playground-e2e
    cd playground && bun run lighthouse
    just vscode-ci

ci-bindings:
    just smoke-ffi
    just smoke-extism
    just smoke-go

ci-repo:
    just typos
    just deny
    just audit
    just shear
    just reuse
    just actionlint
    just zizmor
    cargo run -p aozora-xtask -q -- lint coordinates

ci-corpus:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ -n "${AOZORA_CORPUS_ROOT:-}" && -d "$AOZORA_CORPUS_ROOT" ]] \
      || { echo "AOZORA_CORPUS_ROOT must name the pinned corpus checkout" >&2; exit 2; }
    just audit-gate
    just corpus-sweep
    just verbatim-gate
    just render-leak-gate
    just render-correctness-gate
    just alloc-gate
    just incremental-speedup-gate

ci-perf:
    just perf-gate

ci-release:
    just publish-check
    just semver
    just artifact-crates
    just artifact-size-gate

ci-fuzz:
    #!/usr/bin/env bash
    set -euo pipefail
    for crate in pipeline render encoding; do
      (
        cd "crates/aozora/fuzz/$crate"
        cargo +{{_FUZZ_TOOLCHAIN}} fuzz build --fuzz-dir . --target {{_fuzz_target}}
      )
    done

ci:
    just ci-rust
    just test-wasm
    just ci-web
    just ci-bindings
    just ci-repo
    just ci-perf

# --- changelog ---------------------------------------------------------------

# CHANGELOG.md is owned by release-plz (`[changelog]` in release-plz.toml): it
# maintains the single root changelog inside the Release PR from the
# Conventional-Commits history. There is no `just changelog` recipe — running
# git-cliff by hand would fight release-plz over the file. To preview the next
# changelog locally, run `release-plz update` (writes Cargo.toml / CHANGELOG.md
# in place; discard the spike with `git restore`).

# --- developer workflow helpers ----------------------------------------------

# Start the bacon file-watcher.
# Defaults to the `check` job; pass a job name to pick another, e.g.
# `just watch clippy`. Keybindings: `t` test / `c` clippy / `d` doc /
# `f` failing-only / `esc` previous job / `q` quit / Ctrl-J list jobs.
watch JOB="":
    bacon {{JOB}}

# Watch + re-run clippy on every save (bacon `clippy` job — same lint
# surface as `just clippy`). Fast incremental lint feedback.
watch-lint:
    bacon clippy

# Watch + re-run the nextest suite on every save (bacon `test` job).
watch-test:
    bacon test

# Headless bacon run (no TUI).
# Keeps the watch loop but prints plain lines. Useful for piping output
# (`| tee`) and for sessions without a TTY.
watch-headless JOB="check":
    bacon --headless --job {{JOB}}

# Install git hooks (pre-commit / commit-msg / pre-push).
# Idempotent — re-run safely after lefthook.yml edits or to repair stubs.
hooks:
    lefthook install

# --- playground (React Spectrum + Vite + WASM frontend) ----------------------

# Build the WASM `pkg/` that `vite.config.ts`'s alias targets. Must run
# before `playground-build` (when `.d.ts` is missing or stale).
playground-wasm:
    env CARGO_PROFILE_RELEASE_OPT_LEVEL=z wasm-pack build --target web --release crates/aozora-wasm
    sh -c 'set -eu; wasm=crates/aozora-wasm/pkg/aozora_wasm_bg.wasm; \
        before=$(wc -c < "$wasm"); \
        wasm-opt -Oz --strip-debug --strip-dwarf --vacuum \
            --enable-bulk-memory --enable-mutable-globals \
            --enable-nontrapping-float-to-int "$wasm" -o "$wasm"; \
        after=$(wc -c < "$wasm"); \
        test "$after" -lt "$before"'

# Ensure the playground's prerequisites exist before typecheck / test:
# the wasm `pkg/` that tsc + vite alias `aozora-wasm` to, and the bun
# `node_modules`.
_playground-ensure:
    [ -d crates/aozora-wasm/pkg ] || just playground-wasm
    cd playground && bun install --frozen-lockfile

# Type-check playground TypeScript sources.
playground-typecheck: _playground-ensure
    cd playground && bun run typecheck

# Run the playground and canonical shared-package unit suites with coverage.
playground-test: _playground-ensure
    cd playground && bun run test:coverage

# Combined playground gate: ensure deps once, then typecheck, test, and lint.
# `lint:css` runs stylelint over the playground CSS *and* the canonical
# aozora-notation.css from the repository root.
playground-ci: _playground-ensure
    cd playground && bun run typecheck
    cd playground && bun run test:coverage
    cd playground && bun run lint
    cd playground && bun run lint:css
    cd playground && bun run check:legacy

# Production build of the playground. Regenerates the WASM bundle
# first so the vite alias target is always fresh; `_playground-ensure`
# then guarantees bun deps and correct volume ownership so `vite build`
# can empty `dist`.
playground-build: playground-wasm _playground-ensure
    cd playground && bun run build
    cd playground && bun run check:bundle

# All playground gates in one shot and export the production tree.
playground-all: playground-ci playground-build
    bash -euc 'destination=target/release-ready-build/playground; rm -rf "$destination"; mkdir -p "$destination"; cp -R playground/dist/. "$destination/"; test -s "$destination/index.html"'

# Production Playwright suite. CI installs browser system dependencies before
# calling this recipe; local hosts must provide them through their package manager.
playground-e2e: playground-build
    cd playground && bun run playwright install chromium firefox webkit
    cd playground && bun run playwright test

playground-lighthouse: playground-build
    cd playground && bun run playwright install chromium
    cd playground && bun run lighthouse

# --- VS Code extension (TypeScript, esbuild-bundled) --------------------------
#
# The extension lives under `editors/vscode/` and is its own Bun project.
# Ensure the extension's bun deps exist before a check. A fast lockfile
# verification on a warm checkout; a real install on a fresh one.
_vscode-ensure:
    bash -euc 'cd editors/vscode && bun install --frozen-lockfile'

# Type-check the extension's TypeScript. The tsc half of `vscode-ci`'s
# `check`, split out for the pre-commit hook: biome + the bundle + the test
# suite are more than a commit should pay for.
vscode-typecheck: _vscode-ensure
    bash -euc 'cd editors/vscode && bun run typecheck'

# The extension's full gate, mirroring the CI `vscode` job: biome lint + tsc
# typecheck (`check`), the esbuild bundle (`compile`, which inlines the
# renderer's canonical stylesheet — ADR-0024), and the `node --test` security
# suite.
vscode-ci: _vscode-ensure
    bash -euc 'cd editors/vscode && bun run check && bun run compile && bun run test'

# --- profiling ---------------------------------------------------------------
# samply uses perf_event_open(2) directly. Requires
# /proc/sys/kernel/perf_event_paranoid <= 1; the binary checks and prints
# the fix-up command if not.

# Sample-profile a single corpus document (relative to AOZORA_CORPUS_ROOT).
# Example: just samply-doc 001529/files/50685_ruby_67979/50685_ruby_67979.txt
samply-doc DOC:
    cargo run --release -p aozora-xtask -- samply doc {{DOC}}

# Sample-profile the full corpus parser hot path. REPEAT controls how many
# parse passes the throughput_by_class probe runs after the one-time load,
# so samply has ample parser-bound wall time to attach to. Defaults to 5.
samply-corpus REPEAT="5":
    cargo run --release -p aozora-xtask -- samply corpus {{REPEAT}}

# Sample-profile the HTML render hot path across the full corpus. REPEAT
# controls per-doc render-loop iterations so render frames dominate the
# trace over the per-doc parse warmup. Defaults to 5.
samply-render REPEAT="5":
    cargo run --release -p aozora-xtask -- samply render {{REPEAT}}

# --- trace analysis (post-samply) -------------------------------------------
# `aozora-xtask trace ...` is the analysis half of the samply workflow:
# load a saved .json.gz, symbolicate it (sidecar cache), then run any of
# the bundled analyses (hot / libs / rollup / stacks / compare / flame).
# All commands accept an optional --binary so we can DWARF-resolve the
# right ELF; the sidecar is invalidated if the binary's gnu-build-id no
# longer matches the trace.

# Pre-symbolicate a trace: write <trace>.symbols.json next to it. Subsequent
# `trace hot/rollup/...` calls hit the cache instead of re-walking DWARF.
# BIN defaults to the throughput_by_class profile binary.
trace-cache TRACE BIN="target/release/examples/throughput_by_class":
    cargo run --release -p aozora-xtask -- trace cache {{TRACE}} {{BIN}}

# Top hot leaf frames. TOP controls row count.
trace-hot TRACE TOP="25":
    cargo run --release -p aozora-xtask -- trace hot {{TRACE}} --top {{TOP}}

# Inclusive (self + descendants) hot frames — surfaces entry-point
# functions even when they're not the leaf-most sample.
trace-hot-inclusive TRACE TOP="25":
    cargo run --release -p aozora-xtask -- trace hot {{TRACE}} --top {{TOP}} --inclusive

# Per-library distribution of samples (binary / libc / vdso / …).
trace-libs TRACE:
    cargo run --release -p aozora-xtask -- trace libs {{TRACE}}

# Categorise function names into named buckets via the built-in aozora
# categories (Phase 0/1/2/3/4 + corpus_load + intern + alloc + …).
trace-rollup TRACE:
    cargo run --release -p aozora-xtask -- trace rollup {{TRACE}}

# Print top-K full call stacks containing any frame matching PATTERN.
# Pattern is a regex.
trace-stacks TRACE PATTERN LIMIT="5":
    cargo run --release -p aozora-xtask -- trace stacks {{TRACE}} --pattern {{PATTERN}} --limit {{LIMIT}}

# Diff two traces (BEFORE vs AFTER): show which functions grew, shrank,
# appeared, or disappeared.
trace-compare BEFORE AFTER TOP="25":
    cargo run --release -p aozora-xtask -- trace compare {{BEFORE}} {{AFTER}} --top {{TOP}}

# Emit folded-stack format suitable for flamegraph.pl / inferno-flamegraph.
# Pipe into your flamegraph renderer of choice.
trace-flame TRACE:
    cargo run --release -p aozora-xtask -- trace flame {{TRACE}}

# Remove lefthook git hook stubs.
hooks-uninstall:
    lefthook uninstall

# --- cleanup ------------------------------------------------------------------

# Remove build artifacts.
clean:
    cargo clean --workspace
