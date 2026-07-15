# aozora workspace task runner.
# The ONE entry point for every development operation. Every target runs inside Docker;
# never invoke cargo on the host directly.

set shell := ["bash", "-euo", "pipefail", "-c"]
set dotenv-load := false

# --- internal helpers ---------------------------------------------------------

# Default run prefix for the interactive dev container (TTY attached)
_dev := "docker compose run --rm dev"
# Non-interactive variant for CI-like invocations (no TTY)
_ci  := "docker compose run --rm --no-TTY ci"

# --- metadata -----------------------------------------------------------------

# Default: show this help
default:
    @just --list --unsorted

# First-run setup for a fresh clone: prerequisite checks → build the dev
# image → install git hooks → green test. Idempotent. `./bootstrap` is a
# thin wrapper around this for newcomers who haven't learned `just` yet.
setup:
    #!/usr/bin/env bash
    set -euo pipefail
    command -v docker >/dev/null || { echo "❌ docker not found — install Docker: https://docs.docker.com/get-docker/"; exit 1; }
    docker info >/dev/null 2>&1 || { echo "❌ Docker daemon not running — start Docker and re-run 'just setup'."; exit 1; }
    avail_kb=$(df -Pk . | awk 'NR==2{print $4}')
    if [ "${avail_kb:-0}" -lt 5242880 ]; then echo "⚠️  less than ~5 GB free here — the dev image + cargo volumes want headroom."; fi
    echo "▶ [1/3] Building the dev image (first run ~5 min, cached afterwards)…"
    docker compose build dev
    echo "▶ [2/3] Installing git hooks (lefthook)…"
    just hooks
    echo "▶ [3/3] Verifying the toolchain (just test)…"
    just test
    echo "✅ Setup complete. Try 'just --list' (every recipe) or 'just shell' (dev container)."

# Diagnose the dev environment without changing anything: Docker, the
# dev image, host tools, git hooks, signing, corpus, and profiling
# readiness. The read-only complement to `setup` (which builds and
# installs) — fix with the per-line hints, `just setup`, or `just hooks`.
# Never mutates.
doctor:
    #!/usr/bin/env bash
    set -uo pipefail
    fail=0; warn=0
    ok()   { echo "✅ $1"; }
    bad()  { echo "❌ $1"; echo "      ↳ $2"; fail=$((fail + 1)); }
    note() { echo "⚠️  $1"; echo "      ↳ $2"; warn=$((warn + 1)); }

    echo "── host ──"
    command -v docker >/dev/null \
      && ok "docker installed" \
      || bad "docker not found" "install Docker: https://docs.docker.com/get-docker/"
    docker info >/dev/null 2>&1 \
      && ok "docker daemon running" \
      || bad "docker daemon not running" "start Docker Desktop, or 'sudo systemctl start docker'"
    docker image inspect aozora-dev:local >/dev/null 2>&1 \
      && ok "dev image built (aozora-dev:local)" \
      || note "dev image not built yet" "run 'just setup' (first build ~5 min)"
    command -v mise >/dev/null \
      && ok "mise installed" \
      || note "mise not found (host tool-version manager)" "https://mise.jdx.dev/, then 'mise install'"
    for tool in just lefthook typos committed actionlint; do
      command -v "$tool" >/dev/null \
        && ok "host tool: $tool" \
        || note "host tool missing: $tool" "run 'mise install' (declared in mise.toml)"
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
      && ok "rust-toolchain.toml present (the dev image is canonical)" \
      || note "no rust-toolchain.toml" "the dev image pins the toolchain; host rust is optional"
    { [ -n "${AOZORA_CORPUS_ROOT:-}" ] && [ -d "${AOZORA_CORPUS_ROOT:-}" ]; } \
      && ok "AOZORA_CORPUS_ROOT set ($AOZORA_CORPUS_ROOT)" \
      || note "AOZORA_CORPUS_ROOT unset (corpus sweeps skip)" "export AOZORA_CORPUS_ROOT=\$HOME/aozora-corpus (optional)"
    avail_kb=$(df -Pk . | awk 'NR==2{print $4}')
    [ "${avail_kb:-0}" -ge 5242880 ] \
      && ok "disk headroom (>= 5 GB free)" \
      || note "less than ~5 GB free here" "the dev image + cargo volumes want headroom; try 'docker system prune'"
    if [ -r /proc/sys/kernel/perf_event_paranoid ]; then
      lvl=$(cat /proc/sys/kernel/perf_event_paranoid)
      [ "${lvl:-9}" -le 1 ] \
        && ok "perf_event_paranoid=$lvl (samply profiling ready)" \
        || note "perf_event_paranoid=$lvl (samply needs <= 1)" "echo 1 | sudo tee /proc/sys/kernel/perf_event_paranoid"
    fi

    if docker image inspect aozora-dev:local >/dev/null 2>&1; then
      echo "── container ──"
      {{_dev}} sccache --show-stats >/dev/null 2>&1 \
        && ok "sccache responding in the dev image" \
        || note "sccache not responding" "rebuild the image: 'docker compose build dev'"
    fi

    echo "────"
    if [ "$fail" -gt 0 ]; then
      echo "❌ $fail blocking issue(s), $warn warning(s) — fix the ❌ above (often 'just setup')."
      exit 1
    fi
    echo "✅ no blocking issues ($warn warning(s))."

# --- build/shell --------------------------------------------------------------

# Build all workspace crates.
#
# `aozora-bench` is excluded from every workspace-wide CI gate
# (build / test / coverage / clippy / shear) because it's a
# bench-only harness whose dep tree pulls in `zstd-sys`,
# `criterion`, `addr2line`, `gimli`, `object`, and `ruzstd` —
# adding ~100 s of cold-cache compile time that no other crate in
# the workspace needs. Bench runs go through `just bench`, which
# explicitly invokes `cargo bench --workspace` and gets the full
# tree on demand.
build:
    {{_dev}} cargo build --workspace --exclude aozora-bench --all-targets

# Fastest "does it still compile?" gate. `cargo check` skips codegen,
# so it's the inner-loop signal; `just build` stays the --all-targets
# gate that also links the test / example binaries. Mirrors bacon's
# default `check` job and the MSRV CI job's `cargo check --workspace
# --all-targets`, so the "still compiles?" answer is the same surface
# everywhere.
check:
    {{_dev}} cargo check --workspace --exclude aozora-bench --all-targets

# Build release binaries
build-release:
    {{_dev}} cargo build --release --workspace --exclude aozora-bench

# Drop into an interactive dev shell
shell:
    {{_dev}} bash

# Run the aozora CLI with arbitrary args (`just run check FILE`, etc.)
run *ARGS:
    {{_dev}} cargo run --package aozora-cli --quiet -- {{ARGS}}

# Run a library example from crates/aozora/examples/ (`just example hello`).
# Each example uses only the `aozora` umbrella surface and prints to stdout.
example NAME *ARGS:
    {{_dev}} cargo run -p aozora --example {{NAME}} {{ARGS}}

# --- tests --------------------------------------------------------------------

# Run the full test suite (unit + integration + snapshot).
# `aozora-bench` is excluded — see `build` above for rationale.
test *ARGS:
    {{_dev}} cargo nextest run --workspace --exclude aozora-bench --all-targets {{ARGS}}

# Run only the tests whose name matches FILTER — the single-test inner
# loop. Uses nextest's filterset DSL: a bare string is a substring
# match, wrap it in slashes for a regex. Extra nextest flags pass
# through after FILTER.
#   just t ruby                # every test whose name contains "ruby"
#   just t '/ruby|bouten/'     # regex
#   just t ruby --no-capture   # forward nextest flags
t FILTER *ARGS:
    {{_dev}} cargo nextest run --workspace --exclude aozora-bench -E 'test({{FILTER}})' {{ARGS}}

# Run doctests (nextest skips these by design)
test-doc:
    {{_dev}} cargo test --workspace --doc

# Doctests for the umbrella crate with its optional features enabled
# (wire / cst / query / …), so feature-gated rustdoc examples are
# verified too. `just test-doc` stays feature-light for speed; run this
# before a release or after touching a feature-gated public example.
test-doc-all:
    {{_dev}} cargo test -p aozora --doc --all-features

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
snapshot-update:
    {{_dev}} env INSTA_UPDATE=always cargo nextest run --workspace --all-targets

# Phase K3 — byte-identical render gate. Loads aozora-conformance
# fixtures and asserts current parse → render output matches golden
# files. Set UPDATE_GOLDEN=1 to refresh after intentional output
# change.
render-gate:
    {{_dev}} cargo test -p aozora-conformance --test render_gate

# Refresh aozora-conformance golden files. Use after intentional
# renderer output changes; commit the resulting fixture diff.
render-gate-update:
    {{_dev}} env UPDATE_GOLDEN=1 cargo test -p aozora-conformance --test render_gate

# Phase L1 — regenerate the wire JSON Schema artefacts under
# crates/aozora-conformance/json/. Run after touching any wire struct
# or `aozora::json::SCHEMA_VERSION`; commit the resulting diff so
# `schema-check` (drift gate) stays green.
schema:
    {{_dev}} cargo run -p aozora-xtask -q -- schema dump

# Phase L1 / L4 — drift gate: fail if the on-disk wire schemas
# disagree with the live wire structs. Wired into the `drift-gate`
# CI job; run locally before pushing if you touched wire types.
schema-check:
    {{_dev}} cargo run -p aozora-xtask -q -- schema check

# Phase L2 — regenerate crates/aozora-wasm/types/aozora_types.d.ts
# from the live enums + wire structs. Commit the diff so
# `types-check` stays green.
types:
    {{_dev}} cargo run -p aozora-xtask -q -- types ts

# Phase L2 / L4 — drift gate: fail if the committed
# aozora_types.d.ts disagrees with fresh codegen. Wired into the
# `drift-gate` CI job.
types-check:
    {{_dev}} cargo run -p aozora-xtask -q -- types check

# Generate per-language wire types (Go / …) from the committed wire JSON
# Schema via quicktype — one generator, every host-SDK language. Writes
# `crates/aozora-<lang>/…`; commit the diff so `types-langs-check` stays
# green. quicktype + gofmt ship in the dev image.
types-langs:
    {{_dev}} cargo run -p aozora-xtask -q -- types langs

# Drift gate for the per-language wire types. Wired into `drift-gate`.
types-langs-check:
    {{_dev}} cargo run -p aozora-xtask -q -- types langs-check

# Regenerate the committed tree-sitter parser
# (crates/tree-sitter-aozora/src/{parser.c,grammar.json,node-types.json})
# from grammar.js via the pinned tree-sitter CLI. Run after an intentional
# grammar.js edit; commit the diff so `grammar-check` (drift gate) stays green.
grammar:
    {{_dev}} cargo run -p aozora-xtask -q -- conformance grammar --update

# Drift gate: fail if the committed tree-sitter parser has drifted from a
# fresh `tree-sitter generate` of grammar.js. Wired into `drift-gate`.
grammar-check:
    {{_dev}} cargo run -p aozora-xtask -q -- conformance grammar --check

# Drift gate: fail if release-plz.toml's `changelog_include` has drifted
# from the workspace's publishable members (a crate added to the
# workspace but not to that list drops out of the aggregated CHANGELOG),
# or if a manifest breaks the publish-path hygiene rules the root
# Cargo.toml states in prose: path-only internal dev-deps, and no
# registry version on a `publish = false` member.
# Offline — never contacts crates.io. Wired into `drift-gate`.
publish-check:
    {{_dev}} cargo run -p aozora-xtask -q -- publish check

# Drift gate: rust-toolchain.toml's channel (the DEV toolchain) and
# Cargo.toml's rust-version (the PUBLIC CONTRACT) are two authorities
# holding deliberately different numbers (ADR-0034). Fail if a pin follows
# the wrong one, if a maintained doc names a Rust version outside
# contrib/msrv.md, if a README hand-writes the MSRV badge, or if the
# contract drifts within six months of the channel. Wired into `drift-gate`.
msrv-check:
    {{_dev}} cargo run -p aozora-xtask -q -- msrv check

# Verify the declared MSRV actually builds — the local mirror of CI's
# `msrv` job. Runs on the HOST, not the dev image: the image ships
# rust-toolchain.toml's channel (latest stable, NOT the MSRV) and has no
# way to downgrade. Reads the version from Cargo.toml so it cannot drift
# from the contract it checks. See contrib/msrv.md to re-measure the floor.
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

# Phase L4 — bundled drift gate. Equivalent to the CI `drift-gate`
# job: schema + types + tree-sitter grammar + publish ledger + MSRV pins
# in one shot. Use locally before pushing.
#
# Inlined as a single `docker compose run` rather than a recipe-deps
# chain (`drift-gate: schema-check types-check grammar-check`) so every
# check shares one container start. The previous form burned a full
# container bootstrap (rustup channel sync + components download, ~22 s)
# per CI invocation for each check; the bash -c form runs the later xtask
# invocations against an already-warm container with the xtask binary
# cached in `target/`.
drift-gate:
    {{_dev}} bash -c 'set -euo pipefail; cargo run -p aozora-xtask -q -- schema check && cargo run -p aozora-xtask -q -- types check && cargo run -p aozora-xtask -q -- types langs-check && cargo run -p aozora-xtask -q -- conformance grammar --check && cargo run -p aozora-xtask -q -- publish check && cargo run -p aozora-xtask -q -- msrv check'

# Scaffold a new ADR under docs/adr/ from the template: picks the next
# 4-digit number, slugifies the title, stamps today's date, and writes a
# skeleton. Pure host-side file templating — no container needed.
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

# Phase O4 — WPT-style conformance runner. Four passes in one container:
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
    {{_dev}} bash -c 'set -euo pipefail; cargo run -p aozora-xtask -q -- conformance run && cargo run -p aozora-xtask -q -- conformance vectors && cargo run -p aozora-xtask -q -- conformance run --implementation tree-sitter && cargo run -p aozora-xtask -q -- conformance vectors --implementation tree-sitter && cargo test -p aozora-conformance --test works_gate && cargo run -p aozora-xtask -q -- corpus family-coverage'

# Vendor the conformance vectors from the sibling aozora-notation-spec
# repo into spec-vectors/ (the spec is the source of truth). Host-side —
# reaches outside the /workspace bind mount, so it runs directly on the
# host (like `just deps-*` / `just ci`), not in the dev container. Re-run
# after the spec's vectors change and commit the diff. Override the spec
# location with AOZORA_SPEC_REPO.
sync-spec-vectors:
    cargo run -q --release -p aozora-xtask -- spec-vectors sync

# Fail if the vendored spec-vectors/ have drifted from the sibling spec
# repo. Host-side; `--allow-missing` makes it a no-op where the spec isn't
# checked out (cloud CI / dev container), so the vendored copy is
# authoritative there. Runs in `ci-parallel`'s background lane, so vendored
# drift is caught before every push, and by the weekly spec-freshness
# workflow.
verify-spec-vectors:
    cargo run -q --release -p aozora-xtask -- spec-vectors check --allow-missing

# Property-based tests only. Default 128 cases per proptest block
# (AOZORA_PROPTEST_CASES override via aozora-proptest::config). Fast
# enough to live in `just ci` — see `just prop-deep` for a stress run.
prop:
    {{_dev}} cargo nextest run --workspace --all-features --test 'property_*' --run-ignored default

# Deep property sweep — 4096 cases per block, used before cutting a
# release to exercise invariants beyond the default CI budget.
prop-deep:
    {{_dev}} bash -c 'AOZORA_PROPTEST_CASES=4096 cargo nextest run --workspace --all-features --test "property_*" --run-ignored default'

# Walk every document under `AOZORA_CORPUS_ROOT` and check parse +
# round-trip + source-region tiling (#202) + incremental-merge (#237)
# invariants on the public `aozora::Document` surface.
# Bind-mounts the corpus directory into the container at a stable
# path so the test binary reads it from the same location regardless
# of the host path. Runtime-skips with an informational message if
# the env var is unset — this is *not* a failure, just an indication
# that no corpus is configured.
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
    docker compose run --rm \
        -v "$AOZORA_CORPUS_ROOT":/corpus:ro \
        -e AOZORA_CORPUS_ROOT=/corpus \
        dev cargo nextest run --package aozora --test corpus_sweep --test corpus_splice_tiling --test corpus_incremental_merge --no-capture

# Conformance regression gate: fail when the corpus per-file Unknown-
# degradation rate rises above `corpus/baseline.json` — i.e. when a
# change pushed more notation into the `Annotation{Unknown}` catch-all.
# Same corpus bind-mount as `corpus-sweep`; runtime-skips (NOT a failure)
# when AOZORA_CORPUS_ROOT is unset, so a corpus-less machine still
# pushes — the corpus CI job (which checks out P4suta/aozorabunko_text)
# is the backstop.
#
# Usage:
#   export AOZORA_CORPUS_ROOT=$HOME/aozora-corpus
#   just audit-gate
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
    docker compose run --rm \
        -v "$AOZORA_CORPUS_ROOT":/corpus:ro \
        -e AOZORA_CORPUS_ROOT=/corpus \
        dev cargo run -p aozora-xtask -q -- corpus audit-gate --root /corpus --baseline corpus/baseline.json

# Re-capture `corpus/baseline.json` from the current corpus — the
# ratchet step after a recogniser lands and shrinks the Unknown set.
# Lower it on improvement; never raise it to paper over a regression.
audit-gate-update:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; cannot capture a baseline." >&2
        exit 1
    fi
    docker compose run --rm \
        -v "$AOZORA_CORPUS_ROOT":/corpus:ro \
        -e AOZORA_CORPUS_ROOT=/corpus \
        dev cargo run -p aozora-xtask -q -- corpus audit-gate --root /corpus --baseline corpus/baseline.json --update

# Verbatim-provenance gate: fail when any corpus document's
# `Tree::to_source_verbatim()` no longer equals a fresh `sanitize()` of
# its decoded source (the I5 invariant). Binary — one byte of drift
# fails; needs no baseline. Same corpus bind-mount and runtime-skip
# (NOT a failure when AOZORA_CORPUS_ROOT is unset) as `audit-gate`.
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
    docker compose run --rm \
        -v "$AOZORA_CORPUS_ROOT":/corpus:ro \
        -e AOZORA_CORPUS_ROOT=/corpus \
        dev cargo run -p aozora-xtask -q -- corpus verbatim --root /corpus

# Render-leak ratchet gate: fail when per-marker leak counts (notation
# markers surviving into visible rendered HTML) rise above the committed
# `corpus/render-leak-baseline.json`. The enforcing partner of
# `corpus render-audit` (the per-file diagnostic). Needs a corpus.
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
    docker compose run --rm \
        -v "$AOZORA_CORPUS_ROOT":/corpus:ro \
        -e AOZORA_CORPUS_ROOT=/corpus \
        dev cargo run -p aozora-xtask -q -- corpus render-leak-gate --root /corpus --baseline corpus/render-leak-baseline.json

# Re-capture the render-leak baseline (ratchet down after an improvement).
render-leak-gate-update:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; cannot capture a render-leak baseline." >&2
        exit 1
    fi
    docker compose run --rm \
        -v "$AOZORA_CORPUS_ROOT":/corpus:ro \
        -e AOZORA_CORPUS_ROOT=/corpus \
        dev cargo run -p aozora-xtask -q -- corpus render-leak-gate --root /corpus --baseline corpus/render-leak-baseline.json --update

# Render-correctness ratchet gate: fail when per-category structural render
# defects (I-A HTML tags don't balance, I-C emitted aozora-* class absent from
# AOZORA_CLASSES) rise above `corpus/render-correctness-baseline.json`. The
# enforcing partner of `corpus render-correctness` (the per-file diagnostic).
# Needs a corpus; runtime-skips (NOT a failure) when AOZORA_CORPUS_ROOT is unset.
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
    docker compose run --rm \
        -v "$AOZORA_CORPUS_ROOT":/corpus:ro \
        -e AOZORA_CORPUS_ROOT=/corpus \
        dev cargo run -p aozora-xtask -q -- corpus render-correctness-gate --root /corpus --baseline corpus/render-correctness-baseline.json

# Re-capture the render-correctness baseline (ratchet down after a fix).
render-correctness-gate-update:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; cannot capture a render-correctness baseline." >&2
        exit 1
    fi
    docker compose run --rm \
        -v "$AOZORA_CORPUS_ROOT":/corpus:ro \
        -e AOZORA_CORPUS_ROOT=/corpus \
        dev cargo run -p aozora-xtask -q -- corpus render-correctness-gate --root /corpus --baseline corpus/render-correctness-baseline.json --update

# Render-digest ratchet gate: a non-circular distillation of `corpus audit`
# (panic=0, kind presence-floor, gaiji resolution may only improve). Committed
# at `corpus/render-digest.json`; `unknown_shapes_top` is the informational
# worklist for the normalisation layer. Needs a corpus; runtime-skips otherwise.
digest-gate:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; digest-gate skipped (no corpus to walk)."
        exit 0
    fi
    if [[ ! -d "$AOZORA_CORPUS_ROOT" ]]; then
        echo "AOZORA_CORPUS_ROOT=$AOZORA_CORPUS_ROOT is not a directory." >&2
        exit 1
    fi
    docker compose run --rm \
        -v "$AOZORA_CORPUS_ROOT":/corpus:ro \
        -e AOZORA_CORPUS_ROOT=/corpus \
        dev cargo run -p aozora-xtask -q -- corpus digest-gate --root /corpus --baseline corpus/render-digest.json

# Re-capture the render-digest (ratchet after an improvement).
digest-gate-update:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; cannot capture a render-digest." >&2
        exit 1
    fi
    docker compose run --rm \
        -v "$AOZORA_CORPUS_ROOT":/corpus:ro \
        -e AOZORA_CORPUS_ROOT=/corpus \
        dev cargo run -p aozora-xtask -q -- corpus digest-gate --root /corpus --baseline corpus/render-digest.json --update

# Catalogue-sweep ratchet gate: pin the Tier1/Tier2-matched Unknown shape set and
# per-tier resolved-occurrence counts (`corpus/catalogue-coverage.json`). Residue
# may only shrink; a newly-matched shape fails until a human confirms it is a
# genuine near-miss (the zero-FP guard). Needs a corpus; runtime-skips otherwise.
catalogue-sweep-gate:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; catalogue-sweep-gate skipped (no corpus to walk)."
        exit 0
    fi
    if [[ ! -d "$AOZORA_CORPUS_ROOT" ]]; then
        echo "AOZORA_CORPUS_ROOT=$AOZORA_CORPUS_ROOT is not a directory." >&2
        exit 1
    fi
    docker compose run --rm \
        -v "$AOZORA_CORPUS_ROOT":/corpus:ro \
        -e AOZORA_CORPUS_ROOT=/corpus \
        dev cargo run -p aozora-xtask -q -- corpus catalogue-sweep-gate --root /corpus --baseline corpus/catalogue-coverage.json

# Re-capture the catalogue-coverage baseline (ratchet after vetting a near-miss).
catalogue-sweep-gate-update:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; cannot capture catalogue coverage." >&2
        exit 1
    fi
    docker compose run --rm \
        -v "$AOZORA_CORPUS_ROOT":/corpus:ro \
        -e AOZORA_CORPUS_ROOT=/corpus \
        dev cargo run -p aozora-xtask -q -- corpus catalogue-sweep-gate --root /corpus --baseline corpus/catalogue-coverage.json --update

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
    docker compose run --rm \
        -v "$AOZORA_CORPUS_ROOT":/corpus:ro \
        -e AOZORA_CORPUS_ROOT=/corpus \
        dev cargo run -p aozora-xtask -q -- corpus select-works --root /corpus

# Vendor the works named in `works-selection.toml` into `fixtures/works/` and
# seed their golden HTML. Run after `works-select` (and any manual slug edits).
works-vendor:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; cannot vendor works." >&2
        exit 1
    fi
    docker compose run --rm \
        -v "$AOZORA_CORPUS_ROOT":/corpus:ro \
        -e AOZORA_CORPUS_ROOT=/corpus \
        dev cargo run -p aozora-xtask -q -- corpus vendor-works --root /corpus
    docker compose run --rm -e UPDATE_GOLDEN=1 dev \
        cargo test -p aozora-conformance --test works_gate

# Owned-producer allocation-pressure ratchet (#237 P0.2-real). Measures, via
# dhat around `lex` over the corpus, owned-path allocation count / bytes
# normalized per-file / per-source-byte, and fails when either regresses beyond
# the baseline tolerance (default 3%). Same corpus bind-mount and runtime-skip
# (NOT a failure when AOZORA_CORPUS_ROOT is unset) as `audit-gate`.
#
# Usage:
#   export AOZORA_CORPUS_ROOT=$HOME/aozora-corpus
#   just alloc-gate
alloc-gate:
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
    docker compose run --rm \
        -v "$AOZORA_CORPUS_ROOT":/corpus:ro \
        -e AOZORA_CORPUS_ROOT=/corpus \
        dev cargo run --release -p aozora-bench --example alloc_gate -- --root /corpus --baseline corpus/alloc-baseline.json

# Re-capture the allocation baseline. Ratchet-down on improvement; raise
# only with a PR justification plus a `just throughput` run showing
# wall-clock stays within budget.
alloc-gate-update:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AOZORA_CORPUS_ROOT:-}" ]]; then
        echo "AOZORA_CORPUS_ROOT is not set; cannot capture a baseline." >&2
        exit 1
    fi
    docker compose run --rm \
        -v "$AOZORA_CORPUS_ROOT":/corpus:ro \
        -e AOZORA_CORPUS_ROOT=/corpus \
        dev cargo run --release -p aozora-bench --example alloc_gate -- --root /corpus --baseline corpus/alloc-baseline.json --update

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
    docker compose run --rm \
        -v "$AOZORA_CORPUS_ROOT":/corpus:ro \
        -e AOZORA_CORPUS_ROOT=/corpus \
        dev cargo run --release -p aozora-bench --example throughput

# Instruction-count perf gate (G6). Runs the iai-callgrind micro-benchmarks
# (aozora-bench/benches/perf_gate) under Valgrind's Callgrind, which counts
# CPU *instructions* — deterministic across runs and machines, unlike
# wall-clock (too noisy on shared runners to gate on; see `throughput`).
#
# The FIRST run records a baseline named `perf_gate` and is always green.
# Every later run compares against that baseline and exits non-zero on a
# >10% `Ir` (instructions read) regression on any case (the soft limit is
# baked into the bench's `main!` config). Corpus-free — the bench embeds a
# few vendored 青空文庫 works plus a synthetic annotation-dense buffer — so
# it needs no AOZORA_CORPUS_ROOT. Requires valgrind + iai-callgrind-runner,
# both baked into the dev image.
#
# Runs nightly via .github/workflows/perf.yml (collecting stability data);
# deliberately NOT in ci-parallel / pre-push yet — a per-PR promotion waits
# on that stability data (see the workflow header).
#
# `--allow-aslr`: iai-callgrind otherwise disables ASLR via `setarch -R`,
# whose `personality(2)` call the container's default seccomp profile blocks
# ("Operation not permitted"). Instruction counts are ASLR-independent, so
# leaving ASLR on is harmless here and keeps the recipe container-native (no
# --privileged / seccomp=unconfined).
perf-gate:
    {{_dev}} cargo bench -p aozora-bench --bench perf_gate -- --save-baseline=perf_gate --allow-aslr=true

# --- fuzzing -----------------------------------------------------------------
#
# cargo-fuzz harnesses live under `crates/<crate>/fuzz/` as
# nightly-only sub-crates outside the main workspace (so the workspace
# build doesn't pull libfuzzer-sys). Targets currently registered:
#
#   aozora-pipeline / lex
#   aozora-pipeline / classify
#   aozora-pipeline / ffi_no_abort
#   aozora-render   / render_html
#   aozora-render   / serialize_round_trip
#   aozora-render   / catalogue_normalization
#   aozora-encoding / decode_sjis
#
# Workflow:
#   1. `just fuzz-quick CRATE TARGET`    (60 s) — inner-loop smoke
#   2. `just fuzz-deep  CRATE TARGET`    (5 min) — release pre-flight
#   3. `just fuzz-marathon CRATE TARGET` (15 min) — strongest soak
#   4. On crash, `just fuzz-triage CRATE TARGET` prints just the panic
#      block (panic line + diagnostic context) for every artifact under
#      crates/<crate>/fuzz/artifacts/<target>/. No manual repro loop.
#   5. `just fuzz-promote CRATE TARGET ARTIFACT` lifts an artifact into
#      crates/<crate>/tests/fuzz_regressions/<target>/ so the
#      `tests/fuzz_regressions.rs` integration test replays it on every
#      `just test` run — no nightly required for the regression case.
#   6. `just fuzz-status` is the at-a-glance count of pending crashes
#      vs pinned regressions per target.
#
# See `docs/fuzz-workflow.md` for the long-form description.

# cargo-fuzz is binstalled as a musl-static binary (Dockerfile), so its
# built-in default `--target` is its own musl triple — for which no std ships
# in the nightly image and with which ASan (dynamic-only on Linux) is
# incompatible, so every sanitized build fails before a single fuzz iteration.
# Pin the harnesses to the host gnu triple, whose std the nightly toolchain
# does carry, so `just fuzz-*` actually builds and runs in the canonical image.
_fuzz_target := "x86_64-unknown-linux-gnu"

# Run an arbitrary fuzz target with arbitrary args (escape hatch — the caller
# supplies TARGET, the `--target` triple (see `_fuzz_target` above), and any
# libFuzzer args). The gated recipes below inject `--target` for you.
fuzz CRATE *ARGS:
    {{_dev}} bash -c 'cd crates/{{CRATE}}/fuzz && cargo +nightly fuzz run {{ARGS}}'

# 60-second smoke fuzz — fits inside a development inner loop.
fuzz-quick CRATE TARGET:
    {{_dev}} bash -c 'cd crates/{{CRATE}}/fuzz && cargo +nightly fuzz run --target {{_fuzz_target}} {{TARGET}} -- -max_total_time=60'

# 5-minute deep fuzz — the gate to clear before tagging a release.
fuzz-deep CRATE TARGET:
    {{_dev}} bash -c 'cd crates/{{CRATE}}/fuzz && cargo +nightly fuzz run --target {{_fuzz_target}} {{TARGET}} -- -max_total_time=300'

# 15-minute marathon fuzz — the strongest single-target soak we run by
# hand. Reach for this after a clean fuzz-deep cycle when you want to
# push the corpus another order of magnitude.
fuzz-marathon CRATE TARGET:
    {{_dev}} bash -c 'cd crates/{{CRATE}}/fuzz && cargo +nightly fuzz run --target {{_fuzz_target}} {{TARGET}} -- -max_total_time=900'

# Run every registered fuzz target in turn for 60 s each.
fuzz-all-quick:
    just fuzz-quick aozora-pipeline lex
    just fuzz-quick aozora-pipeline classify
    just fuzz-quick aozora-pipeline ffi_no_abort
    just fuzz-quick aozora-render render_html
    just fuzz-quick aozora-render serialize_round_trip
    just fuzz-quick aozora-render catalogue_normalization
    just fuzz-quick aozora-encoding decode_sjis

# Run every registered fuzz target in turn for 5 min each — the
# release pre-flight gate.
fuzz-all-deep:
    just fuzz-deep aozora-pipeline lex
    just fuzz-deep aozora-pipeline classify
    just fuzz-deep aozora-pipeline ffi_no_abort
    just fuzz-deep aozora-render render_html
    just fuzz-deep aozora-render serialize_round_trip
    just fuzz-deep aozora-render catalogue_normalization
    just fuzz-deep aozora-encoding decode_sjis

# Reproduce every artifact under crates/<crate>/fuzz/artifacts/<target>/
# and print the panic block (panicked-at line + diagnostic context the
# fuzz target embeds in the panic message). Exit status is the count of
# artifacts that still crash so it can drive a CI gate.
fuzz-triage CRATE TARGET:
    #!/usr/bin/env bash
    set -euo pipefail
    crate="{{CRATE}}"
    target="{{TARGET}}"
    art_dir="crates/${crate}/fuzz/artifacts/${target}"
    if [[ ! -d "$art_dir" ]]; then
        echo "fuzz-triage: no artifacts for ${crate} / ${target}"
        exit 0
    fi
    failed=0
    for art in $(find "$art_dir" -type f \( -name 'crash-*' -o -name 'leak-*' -o -name 'oom-*' \) | sort); do
        # `cargo fuzz run` resolves relative paths against the fuzz
        # crate's own directory (we cd into crates/<crate>/fuzz before
        # invoking it), so strip the prefix accordingly.
        rel="${art#crates/${crate}/fuzz/}"
        echo "==> ${art}"
        out=$({{_dev}} bash -c "cd crates/${crate}/fuzz && cargo +nightly fuzz run --target {{_fuzz_target}} ${target} ${rel} 2>&1" || true)
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
# `just test` run. The mv goes through the dev container because the
# artifact was written by libFuzzer running as root inside it — host
# permissions can't unlink it.
fuzz-promote CRATE TARGET ARTIFACT:
    #!/usr/bin/env bash
    set -euo pipefail
    src="crates/{{CRATE}}/fuzz/artifacts/{{TARGET}}/{{ARTIFACT}}"
    dst_dir="crates/{{CRATE}}/tests/fuzz_regressions/{{TARGET}}"
    if [[ ! -f "$src" ]]; then
        echo "fuzz-promote: artifact not found: $src" >&2
        exit 1
    fi
    {{_dev}} bash -c "mkdir -p '$dst_dir' && mv '$src' '$dst_dir/{{ARTIFACT}}'"
    echo "promoted ${src} -> ${dst_dir}/{{ARTIFACT}}"

# At-a-glance health: per-target pending crashes vs pinned regressions.
# Nothing here invokes nightly so it stays cheap and shell-friendly.
fuzz-status:
    #!/usr/bin/env bash
    set -euo pipefail
    targets=(
        "aozora-pipeline lex"
        "aozora-pipeline classify"
        "aozora-pipeline ffi_no_abort"
        "aozora-render render_html"
        "aozora-render serialize_round_trip"
        "aozora-render catalogue_normalization"
        "aozora-encoding decode_sjis"
    )
    printf "%-22s  %-22s  %-10s  %-12s\n" crate target pending_crashes pinned_regressions
    printf "%-22s  %-22s  %-10s  %-12s\n" ---------------------- ---------------------- ---------- ------------
    for entry in "${targets[@]}"; do
        crate="${entry% *}"
        target="${entry#* }"
        crashes=0
        regressions=0
        art_dir="crates/${crate}/fuzz/artifacts/${target}"
        reg_dir="crates/${crate}/tests/fuzz_regressions/${target}"
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
    {{_dev}} cargo bench --workspace {{ARGS}}

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
    {{_dev}} cargo bench --workspace -- --save-baseline {{NAME}}

# Re-run benches and compare against an earlier saved baseline.
# Criterion prints mean / stddev / p-value per bench; a regression
# > 5% with `change.p_value < 0.05` is a meaningful signal worth
# investigating before cutting a release.
bench-compare NAME="main":
    {{_dev}} cargo bench --workspace -- --baseline {{NAME}}

# Heap-allocation profile (dhat) of a 2 MiB synthetic parse + render:
# total allocations + peak bytes, plus dhat-heap.json for dh_view. dhat
# needs no perf_event_open, so (unlike `samply-*`) it runs in Docker.
dhat:
    {{_dev}} cargo run --release -p aozora-bench --example dhat_parse

# Corpus-free small-doc parse+render latency percentiles (p50/p90/p99/max)
# over a synthetic buffer. The deep per-phase, corpus-driven view is the
# `latency_histogram` example.
latency:
    {{_dev}} cargo run --release -p aozora-bench --example latency_synthetic

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
    {{_dev}} sh -c 'rm -rf "${CARGO_TARGET_DIR:-target}/llvm-cov-target"'
    {{_dev}} cargo llvm-cov nextest \
        --workspace --exclude aozora-bench \
        --ignore-filename-regex '{{_COV_IGNORE}}' \
        --fail-under-regions {{_COV_FLOOR}}

# Run aozora-lsp's `internals`-gated integration suites (smoke / guardian /
# concurrent_lsp / concurrency_regressions / shuttle / property_invariants /
# differential / snapshots / fuzz_regressions). `coverage` runs nextest with
# DEFAULT features, so these — gated on `required-features = ["internals"]` —
# are skipped there and would otherwise only ever be COMPILED (by
# `clippy-strict --all-targets --all-features`), never run. That gap let a
# stale assertion rot undetected; this gate closes it.
#
# Scoped to `-p aozora-lsp` on purpose: a blanket `--workspace
# --all-features` would also flip on feature flags whose tests need external
# setup — e.g. aozora-extism's `host-smoke`, which loads a pre-built wasm
# artifact produced by `extism-build` / `smoke-extism` and fails on a clean
# runner. aozora-lsp is the only crate whose feature-gated integration tests
# have no dedicated gate.
test-internals:
    {{_dev}} cargo nextest run -p aozora-lsp --all-features

# HTML coverage report for local inspection. No threshold — intended
# for opening `coverage/html/index.html` in a browser.
coverage-html:
    {{_dev}} cargo llvm-cov nextest \
        --workspace --exclude aozora-bench \
        --ignore-filename-regex '{{_COV_IGNORE}}' \
        --html --output-dir coverage/html

# Branch-level coverage report (requires nightly for `--branch` support).
# Informational only — no threshold. Use to surface uncovered conditionals
# when working a specific file toward C1 100%.
coverage-branch:
    {{_dev}} cargo +nightly llvm-cov nextest \
        --branch \
        --workspace --exclude aozora-bench \
        --ignore-filename-regex '{{_COV_IGNORE}}'

# --- mutation testing --------------------------------------------------------

# Assertion-strength gate (cargo-mutants). Mutates the source and checks
# the suite CATCHES each change — the complement region coverage can't
# give: coverage proves a line ran, mutation proves a wrong result would
# fail a test (ADR-0031). Report-only today: read `mutants.out/`, write
# tests to kill surviving mutants, and `#[mutants::skip]` (with a reason)
# the equivalent / unreachable ones.
#
#   just mutants -p aozora-cst                          # one crate (fast)
#   just mutants --in-diff <(git diff origin/main)      # only changed lines
#
# Runs in a DEDICATED target dir on the persistent cargo-target volume
# with CARGO_INCREMENTAL=1: the compose file pins it to 0 so sccache can
# cache, but mutation rebuilds every mutant serially in one scratch tree,
# so incremental reuse is the win and sccache (which cannot cache
# incremental builds) is dropped via RUSTC_WRAPPER=. The `/mutants`
# subdir keeps these incremental artefacts isolated from the main
# sccache'd `/cargo/target/debug`, so neither build clobbers the other.
# Config: repo-root `mutants.toml` via --config (`.gitignore` hides the
# tool's default `.cargo/` location).
mutants *ARGS:
    docker compose run --rm \
        -e CARGO_TARGET_DIR=/cargo/target/mutants \
        -e CARGO_INCREMENTAL=1 \
        -e RUSTC_WRAPPER= \
        dev cargo mutants --config mutants.toml {{ARGS}}

# Host-native mutation sweep — the FAST inner loop for reinforcing a crate.
# Same cargo-mutants (pinned 27.1.0 via mise) + nextest + rust channel as the
# Docker `just mutants` above, so it enumerates the identical mutant set and
# the baseline it produces holds in CI (see ADR-0031 "host lane"). Docker
# `just mutants` stays the authoritative CI/parity mirror; this trades the
# container's locale-pinned reproducibility for wall-clock — it skips the
# compose spin-up and drives cargo-mutants' own `-j` parallelism natively.
#
#   just mutants-host -p aozora-scan              # one crate, 4-way parallel
#   MUTANTS_JOBS=6 just mutants-host -p aozora-pipeline   # override the fan-out
#   just mutants-host --in-diff <(git diff origin/main)  # only changed lines
#
# A dedicated `target/mutants-host` keeps these serial-rebuild artefacts out
# of the normal host `target/` (and never touches the Docker volume). The
# machine-readable report still lands in the git-ignored repo-root
# `mutants.out/`, identical in shape to the Docker lane's.
mutants-host *ARGS:
    CARGO_TARGET_DIR=target/mutants-host \
        cargo mutants --config mutants.toml -j "${MUTANTS_JOBS:-4}" {{ARGS}}

# --- lint / static analysis ---------------------------------------------------

# Run all lints (fmt + clippy + typos + strict-code + doc)
lint: fmt-check clippy typos strict-code doc

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
    {{_dev}} env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items --all-features --jobs 1

# Forbid patterns that hide bugs or introduce unstable/unsafe surface in our
# own crates. Every check is defensive — each represents a pattern we have
# decided IS a bug-source and want rejected at the gate rather than fought
# later in code review.
strict-code:
    #!/usr/bin/env bash
    set -euo pipefail
    shopt -s globstar
    files=(crates/**/*.rs)

    # Crates that legitimately need an unsafe escape hatch — they
    # are still linted by `#[deny(unsafe_op_in_unsafe_fn)]` and a
    # crate-local `#![allow(unsafe_code)]` (with reason=) attribute,
    # so the compiler still gates each unsafe block:
    #
    #   - aozora-ffi   : C ABI bindings (`unsafe extern "C"`)
    #   - tree-sitter-aozora : C parser FFI binding — the standard
    #                    tree-sitter-language pattern (`unsafe extern "C"`
    #                    for the generated `tree_sitter_aozora()` symbol,
    #                    `LanguageFn::from_raw` to wrap it)
    #   - aozora-scan  : x86_64 AVX2 intrinsics (SIMD scanner)
    #   - aozora-xtask : dev-tooling binary; `#[allow(reason=...)]`
    #                    for narrow clippy carve-outs is acceptable
    #                    here per Rust 1.81+ stable convention
    #   - */fuzz/*     : cargo-fuzz harnesses are dev-only and never
    #                    shipped; the FFI no-abort target must drive the
    #                    `aozora-ffi` C ABI through `unsafe`. The shipped
    #                    parser core stays fully under the no-unsafe gate.
    #
    # The grep below skips these paths; everything else stays under the
    # universal "no unsafe" gate.
    is_unsafe_exempt() {
        case "$1" in
            crates/aozora-ffi/*|crates/tree-sitter-aozora/*|crates/aozora-scan/*|crates/aozora-xtask/*) return 0 ;;
            crates/*/fuzz/*) return 0 ;;
            *) return 1 ;;
        esac
    }

    check_strict() {
        local label="$1"
        local pattern="$2"
        local hits
        hits=$(grep -nE "$pattern" "${files[@]}" 2>/dev/null || true)
        # Filter out exempt crates.
        local filtered=""
        while IFS= read -r line; do
            [[ -z "$line" ]] && continue
            local path="${line%%:*}"
            if ! is_unsafe_exempt "$path"; then
                filtered+="${line}"$'\n'
            fi
        done <<< "$hits"
        if [[ -n "$filtered" ]]; then
            echo "==> forbidden: $label" >&2
            printf '%s' "$filtered" >&2
            return 1
        fi
    }

    check() {
        local label="$1"
        local pattern="$2"
        local hits
        hits=$(grep -nE "$pattern" "${files[@]}" 2>/dev/null || true)
        if [[ -n "$hits" ]]; then
            echo "==> forbidden: $label" >&2
            echo "$hits" >&2
            return 1
        fi
    }

    failed=0

    # ---- Warning suppression -----------------------------------------------
    # `#[allow(... reason = "...")]` (Rust 1.81+ stable) is the
    # documented "I've considered this lint and overridden it
    # deliberately" idiom and is allowed; bare `#[allow(...)]` without
    # a reason is forbidden. We grep with -A 5 to catch the reason
    # clause when it's on a continuation line, then filter out hits
    # whose surrounding window contains `reason = `.
    #
    # `build.rs` files are excluded because their string literals
    # often contain `#[allow(reason="...")]` snippets that they emit
    # as generated Rust code — they are not actual Rust attributes
    # under strict-code's purview.
    src_files=()
    for f in "${files[@]}"; do
        case "$f" in
            */build.rs) ;;
            *) src_files+=("$f") ;;
        esac
    done
    bare_allow=$(grep -nE -A 5 '^\s*#!?\[allow\(' "${src_files[@]}" 2>/dev/null \
        | awk -F: '
            /#!?\[allow\(/      { capture = 1; window = ""; head = $0 }
            capture              { window = window $0 "\n" }
            capture && /\)\]/    {
                if (window !~ /reason[[:space:]]*=[[:space:]]*"/) {
                    print head
                }
                capture = 0
            }
        ' || true)
    if [[ -n "$bare_allow" ]]; then
        echo '==> forbidden: warning suppression (#[allow] without reason="...")' >&2
        echo "$bare_allow" >&2
        failed=1
    fi
    check 'cfg_attr-wrapped warning suppression' \
        '^\s*#!?\[cfg_attr\([^)]*allow\(' || failed=1

    # ---- Nightly / unstable feature gates ----------------------------------
    check 'nightly feature gate (#[feature] / #![feature])' \
        '^\s*#!?\[feature\(' || failed=1

    # ---- Unsafe code -------------------------------------------------------
    # Every non-exempt crate root has `#![forbid(unsafe_code)]`
    # (checked below); this text-level grep is belt-and-braces for
    # typos that would defeat the compiler gate.
    check_strict 'unsafe code (unsafe fn / unsafe { / unsafe impl / unsafe trait)' \
        '(^|[^a-zA-Z_#])unsafe\s+(fn|impl|trait|\{)' || failed=1

    # ---- Notation slug misspellings (regression guard) --------------------
    # The romaji CSS slugs are centralised in `aozora-spec::RENDER_SLUGS`
    # and machine-checked against their kana reading there. These greps
    # are the cheap last line of defence: `koshogaki` was a misreading of
    # 小書き (こがき → `kogaki`); `choho`/`dan`/`spread` are the pre-Hepburn
    # section-break slugs (now `kaicho`/`kaidan`/`kaimihiraki`). If either
    # reappears anywhere in the source tree, fail loudly.
    check 'misread slug koshogaki (小書き＝こがき → kogaki)' \
        'koshogaki' || failed=1
    check 'stale section-break slug (choho/dan/spread → kaicho/kaidan/kaimihiraki)' \
        'section-break-(choho|dan|spread)' || failed=1

    # ---- Required deny directive -------------------------------------------
    for root in crates/*/src/lib.rs crates/*/src/main.rs; do
        [[ -f "$root" ]] || continue
        if is_unsafe_exempt "$root"; then continue; fi
        if ! grep -q '^#!\[forbid(unsafe_code)\]' "$root"; then
            echo "==> forbidden: crate root missing '#![forbid(unsafe_code)]'" >&2
            echo "  $root" >&2
            failed=1
        fi
    done

    # ---- Toolchain pinning -------------------------------------------------
    if grep -qE '^\s*channel\s*=\s*"(nightly|beta)' rust-toolchain.toml; then
        echo "==> forbidden: rust-toolchain.toml pins a pre-stable channel" >&2
        grep -nE '^\s*channel' rust-toolchain.toml >&2
        failed=1
    fi

    # ---- TODO/FIXME/XXX without an issue reference -------------------------
    todo_hits=$(grep -nE '(^|[^[:alnum:]_])(TODO|FIXME|XXX)([^[:alnum:]_]|$)' "${files[@]}" 2>/dev/null \
        | grep -vE '(#[0-9]+|M[0-9]|issue)' || true)
    if [[ -n "$todo_hits" ]]; then
        echo '==> forbidden: bare TODO/FIXME/XXX without an issue or milestone reference' >&2
        echo "$todo_hits" >&2
        failed=1
    fi

    # ---- println! / eprintln! in library crates ----------------------------
    # Library crates emit observability via `tracing`, not raw print.
    # CLI crates (aozora-cli) and tests/examples/fuzz are exempt.
    # `build.rs` is also exempt: `println!("cargo:rerun-if-changed=...")`
    # is the documented cargo build-script protocol, not a stray
    # debug print — see https://doc.rust-lang.org/cargo/reference/build-scripts.html
    lib_files=(crates/aozora-syntax/**/*.rs crates/aozora-lexer/**/*.rs crates/aozora-lex/**/*.rs crates/aozora-render/**/*.rs crates/aozora-encoding/**/*.rs)
    print_hits=$(grep -nE '(^|[^[:alnum:]_])e?print(ln)?!\s*\(' "${lib_files[@]}" 2>/dev/null \
        | grep -vE '/(tests|benches|examples|fuzz_targets)/|/build\.rs:' || true)
    if [[ -n "$print_hits" ]]; then
        echo '==> forbidden: println! / eprintln! in library crates (use tracing instead)' >&2
        echo "$print_hits" >&2
        failed=1
    fi

    # ---- Aozora purity: no comrak USE in code -----------------------------
    # The aozora repo is the pure 青空文庫記法 layer; the Markdown
    # integration lives in the sibling `afm` repo. Doc-comment prose is
    # exempt (it routinely explains how downstream integrations layer on
    # top), but a `use comrak` import or `comrak::` path means a real
    # dependency leak from the dialect side.
    use_hits=$(grep -nE '^\s*(use|extern crate)\s+comrak\b|\bcomrak::[a-zA-Z_]' "${files[@]}" 2>/dev/null \
        | grep -vE '^[^:]+:[0-9]+:\s*//' || true)
    if [[ -n "$use_hits" ]]; then
        echo '==> forbidden: comrak import / path-expression in aozora source' >&2
        echo "$use_hits" >&2
        failed=1
    fi

    # ---- expect() in pipeline source files (regression gate) ----------
    # Counts every `.expect(` in `crates/aozora-pipeline/src/**` —
    # including test-module bodies, since this is a coarse "no
    # regression" tripwire, not a precise audit. PR 4 of the
    # quality-hardening plan replaced pipeline.rs's state-transition
    # `Option::expect` chain (13 calls) with a field-bound type-state
    # struct, dropping the workspace total from 58 to 50; the
    # remaining 50 are split across genuine bounds checks
    # (`u32::try_from(len).expect("fits per Phase 0 cap")`),
    # locally-justified `next().expect()` after a length check, and
    # in-source `#[cfg(test)] mod tests` assertions. The baseline
    # gates against new state-assertion-style expects landing in
    # production paths.
    #
    # 51 (was 50): the coremodel Format unification (#189) made
    # `FontShift` wrap a `NonZeroI8`, so the classify test module gained
    # one `fs(steps)` data-builder helper bridging an i8 literal to the
    # now-type-safe constructor (`NonZeroI8::new(steps).expect(..)`). A
    # test-data helper, not a production state-assertion — exactly the
    # invariant-in-the-type move this gate rewards.
    expect_files=(crates/aozora-pipeline/src/**/*.rs)
    expect_count=$(grep -hcE '\.expect\(' "${expect_files[@]}" 2>/dev/null \
        | awk '{s+=$1} END {print s+0}')
    expect_baseline=51
    if [[ "$expect_count" -gt "$expect_baseline" ]]; then
        echo "==> forbidden: expect() count in aozora-pipeline source grew" >&2
        echo "    baseline: $expect_baseline, found: $expect_count" >&2
        echo "    Add a property test or refactor to lift the invariant into the type" >&2
        echo "    instead of pushing it to runtime. See PR 4 of the hardening plan." >&2
        failed=1
    fi

    if [[ $failed -ne 0 ]]; then
        echo "" >&2
        echo "strict-code check failed. Refactor the offending sites; do not silence." >&2
        exit 1
    fi
    echo "strict-code: clean (expect-count $expect_count / baseline $expect_baseline)"

# Every publishable crate (one that is NOT `publish = false`) must ship a
# README.md — otherwise its crates.io page renders empty (F9). And that README
# may not carry repo-relative links: crates.io / docs.rs resolve links against
# nothing, so `](../foo)` / `](./foo)` (and HTML `href`/`src="./…"`) render as
# dead 404s on the crate page. Absolute https URLs only. The repo-root README.md
# / README.ja.md are the GitHub landing pages, not crate readmes, so their
# relative links are fine and they are not scanned. Pure grep/bash, so it runs
# on the bare host (no dev image) — matching the `readme-gate` CI job.
readme-gate:
    #!/usr/bin/env bash
    set -euo pipefail
    missing=""
    relative=""
    for manifest in crates/*/Cargo.toml; do
        dir=$(dirname "$manifest")
        # Skip crates opted out of crates.io publication.
        if grep -qE '^\s*publish\s*=\s*false' "$manifest"; then
            continue
        fi
        readme="$dir/README.md"
        if [[ ! -f "$readme" ]]; then
            missing+="  ${dir#crates/}"$'\n'
            continue
        fi
        hits=$(grep -nE '\]\(\.\.?/|(href|src)="\.\.?/' "$readme" || true)
        if [[ -n "$hits" ]]; then
            relative+="  == $readme"$'\n'"$hits"$'\n'
        fi
    done
    fail=0
    if [[ -n "$missing" ]]; then
        echo "==> publishable crates missing a README.md (crates.io page would be empty):" >&2
        printf '%s' "$missing" >&2
        fail=1
    fi
    if [[ -n "$relative" ]]; then
        echo "==> repo-relative links in a published crate README" >&2
        echo "    (they 404 on crates.io / docs.rs — use absolute https URLs):" >&2
        printf '%s' "$relative" >&2
        fail=1
    fi
    [[ $fail -eq 0 ]] || exit 1
    echo "readme-gate: clean"

# Format check (no-write): Rust (rustfmt) + TOML (taplo, taplo.toml policy)
fmt-check:
    {{_dev}} cargo fmt --all -- --check
    {{_dev}} taplo fmt --check

# Auto-format (writes): Rust (rustfmt) + TOML (taplo, taplo.toml policy)
fmt:
    {{_dev}} cargo fmt --all
    {{_dev}} taplo fmt

# Clippy — lint groups (pedantic/nursery/cargo) and carve-outs are owned
# entirely by `[workspace.lints]` in Cargo.toml. Passing `-W clippy::<group>`
# here would re-enable the whole group at CLI priority and silently undo
# per-lint allow carve-outs (e.g. `redundant_pub_crate`). Keep the CLI
# surface to `-D warnings` only.
# `--lib --bins --tests` instead of `--all-targets` skips the
# example / bench targets. Several crates (aozora-pipeline,
# aozora-syntax, aozora-scan) declare `[[bench]]` entries that pull
# `criterion`'s entire dep tree (zstd / object / addr2line / gimli)
# into the clippy build for no real lint signal — clippy on a bench
# harness almost never fires anything that wouldn't fire on the lib
# it benches. Bench breakage gets caught the moment you actually
# run `just bench`, where it should.
clippy:
    {{_dev}} cargo clippy --workspace --exclude aozora-bench --lib --bins --tests --all-features -- -D warnings

# Strict variant: full `--all-targets` (lib + bins + tests + examples
# + benches), and the bench crate is no longer excluded. This is the
# AUTHORITATIVE lint surface — it matches GitHub's `lint (clippy-strict)`
# cell exactly and is run by the pre-push gate (`ci-parallel`), so a
# doc_markdown / missing_docs slip in a bench or example target is
# caught locally before the push, not by CI. The per-commit hook stays
# on the lighter `clippy` (no bench dep tree) for fast feedback.
clippy-strict:
    {{_dev}} cargo clippy --workspace --all-targets --all-features -- -D warnings

# wasm32-target clippy for the wasm-bindgen / Extism plugin crates.
# The host `clippy` recipe builds for the NATIVE target, so it never
# compiles the `#[cfg(target_arch = "wasm32")]` binding modules — their
# lint debt would otherwise stay invisible to every host gate. This
# recipe closes that gap by linting the two wasm32 crates on their real
# target. The dev image ships wasm32-unknown-unknown (see `extism-build`).
clippy-wasm:
    {{_dev}} cargo clippy --target wasm32-unknown-unknown -p aozora-wasm -p aozora-extism -- -D warnings

# Thorough local lint — the --all-targets clippy surface (bench /
# example targets included) plus fmt / typos / strict-code / doc. Run
# before cutting a release or after touching a bench / example target.
# The per-commit hook runs only the lighter `clippy`; the pre-push gate
# (`ci-parallel`) runs `clippy-strict` + `clippy-wasm`, matching CI's
# authoritative --all-targets + wasm32 lint cells.
lint-full: fmt-check clippy-strict typos strict-code doc

# Typo check
typos:
    {{_dev}} typos

# Dependency linting (licenses, advisories, bans)
deny:
    {{_dev}} cargo deny check

# RustSec advisory scan
audit:
    {{_dev}} cargo audit

# Unused-dependency scan. cargo-shear is stable (no nightly), fast, and
# also flags unlinked source files; it replaces the former nightly
# cargo-udeps gate. Covers the whole workspace — `aozora-bench` included
# — in a single pass, so no separate bench run is needed. Intentional
# optional deps are carved out via `[package.metadata.cargo-shear]`.
shear:
    {{_dev}} cargo shear

# Semver break detection against the crates.io baseline. cargo-semver-checks
# hard-aborts the whole run on the first publishable crate with no registry
# baseline, so exclude the crates that have never been published — until their
# first crates.io release, at which point they drop off this list. (Bin-only
# and `publish = false` members are skipped automatically; a real break makes
# the run exit non-zero, which is expected on a breaking release.)
semver:
    {{_dev}} cargo semver-checks check-release --workspace \
        --exclude aozora-buildstamp \
        --exclude aozora-fmt \
        --exclude aozora-lsp \
        --exclude tree-sitter-aozora

# --- dependency follow-up (local-only, no remote CI) -------------------------
# Policy: workspace deps track @latest. The mechanism is purely local —
# `just deps-check` runs the full dependency-health gate (outdated +
# audit + deny), `just upgrade` bumps Cargo.toml to the latest
# compatible versions, and a systemd user timer (see
# `deps-timer-install`) runs `just deps-check` weekly so new advisories
# surface even on quiet branches.

# `target/.deps-check.timestamp` is the last-success marker that
# `deps-status` reads. Written under `target/` (Docker-volume-mounted
# so host can read it) and intentionally ephemeral — `cargo clean`
# wipes it, which prompts a fresh `deps-check`.
_deps_marker := "target/.deps-check.timestamp"

# Show out-of-date workspace deps (root deps only — transitive bumps
# are noise unless they break something). Exit 0 even when something
# is outdated; this recipe is for inspection, not for gating.
outdated:
    {{_dev}} cargo outdated --workspace --root-deps-only --depth 2 --exit-code 0

# Bump every workspace dep to the latest semver-compatible version
# and re-resolve `Cargo.lock`. Safe to run anytime; rejects
# major-version bumps (use `upgrade-incompat` for those, opt-in,
# review-required).
upgrade:
    {{_dev}} cargo upgrade --workspace --pinned --recursive
    {{_dev}} cargo update --workspace
    @echo "Lockfile updated. Run 'just ci' before committing to verify."

# Bump every workspace dep including major-version (incompatible)
# bumps. Always review the Cargo.toml diff afterwards — major bumps
# are API breaks by definition, and the build / test gate is the
# only thing that catches breakage.
upgrade-incompat:
    {{_dev}} cargo upgrade --workspace --incompatible allow --recursive
    {{_dev}} cargo update --workspace
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
# `WorkingDirectory=$REPO`). Idempotent. Runs on the host, not in
# the dev container, because `systemctl --user` only makes sense on
# the host.
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

# PGO (+ optional BOLT) release build. Needs cargo-pgo installed
# (`cargo install cargo-pgo`) and AOZORA_CORPUS_ROOT pointing at a
# real Aozora corpus checkout. See scripts/pgo-build.sh for details.
# Runs on the host (not in the dev container) because cargo-pgo +
# llvm-bolt expect direct access to the host's profiling data.
pgo:
    bash scripts/pgo-build.sh

# C ABI smoke test — builds aozora-ffi as cdylib, compiles the C
# harness against it, runs end-to-end.
smoke-ffi:
    bash crates/aozora-ffi/tests/c_smoke/run.sh

# Build the single portable `aozora.wasm` Extism plugin (the polyglot
# transport hub) and copy it to crates/aozora-extism/dist/. Every
# language with an Extism host SDK loads this ONE artifact — there is no
# per-(OS × arch) native build matrix the way the aozora-ffi C ABI needs.
# The dev image ships binaryen's `wasm-opt` (see Dockerfile); the recipe
# still degrades gracefully to an unoptimized artifact if a custom image
# lacks it. See ADR-0006.
extism-build:
    {{_dev}} cargo build --release --target wasm32-unknown-unknown -p aozora-extism
    {{_dev}} sh -c 'mkdir -p crates/aozora-extism/dist \
        && cp "${CARGO_TARGET_DIR:-target}/wasm32-unknown-unknown/release/aozora_extism.wasm" crates/aozora-extism/dist/aozora.wasm \
        && (command -v wasm-opt >/dev/null 2>&1 \
            && wasm-opt -O3 --enable-bulk-memory --enable-mutable-globals \
                crates/aozora-extism/dist/aozora.wasm -o crates/aozora-extism/dist/aozora.wasm \
            && echo "wasm-opt applied" \
            || echo "wasm-opt not present — shipping unoptimized artifact")'

# End-to-end cross-language ABI check (the Extism analogue of smoke-ffi):
# build the plugin, then load the built aozora.wasm through the Extism
# (Rust) host SDK and assert every export is byte-identical to calling
# aozora::json in-process. The `host-smoke` feature pulls wasmtime, so it
# is opt-in and never burdens `just test` / `just ci`.
smoke-extism: extism-build
    {{_dev}} cargo test -p aozora-extism --features host-smoke --test host_smoke -- --nocapture

# End-to-end Go host SDK check (the Go analogue of smoke-ffi / smoke-extism):
# build the plugin, embed it in the Go package, and run `go test`, which
# loads aozora.wasm through the pure-Go wazero Extism runtime and decodes
# every wire envelope into the quicktype-generated Go structs. Kept out of
# `just ci` (first run needs `go mod download`); run manually / in a job.
smoke-go: extism-build
    {{_dev}} bash -c 'set -euo pipefail; \
        cp crates/aozora-extism/dist/aozora.wasm crates/aozora-go/aozora.wasm; \
        cd crates/aozora-go; \
        unformatted=$(gofmt -l .); \
        if [ -n "$unformatted" ]; then echo "gofmt needs: $unformatted"; exit 1; fi; \
        go vet ./...; \
        go test ./...'

# Python wheel smoke — HOST-side (maturin + a Python interpreter are not
# in the dev image, like smoke-ffi / pgo). Provisions a throwaway venv,
# builds the abi3 wheel, installs it, then runs mypy --strict + pytest.
# Kept out of `just ci` (the dev image can't run it); mirrored by the
# ci.yml `python-wheel` job. Knobs: AOZORA_PY_PYTHON / AOZORA_PY_VENV.
#
# The cross-surface parity gate's Python channel (`tests/test_fixture_parity.py`)
# rides on this pytest run and its `python-wheel` CI mirror — a DOCUMENTED
# `ci-parallel` exception: the dev image ships no Python interpreter, so
# there is no in-container lane for it (same rationale as smoke-ffi).
smoke-py:
    bash scripts/smoke-py.sh

# Cross-surface parity gate — wasm (Node) channel. Builds the wasm-pack
# `--target nodejs` package and walks every render fixture through it,
# asserting each surface (html / serialize / diagnostics / nodes / pairs /
# container_pairs) is byte-identical to the committed golden — the same
# golden the in-process `render_gate` pins. The `--target web` pkg the
# playground consumes is a separate out-dir, so this leaves it untouched.
# Wired into `ci-parallel` (foreground tail, right after `extism-build`)
# and the CI `wasm-build` job (host mirror: two raw steps). The sibling
# CLI / FFI / Python / Go walkers cover the other channels.
parity-wasm:
    {{_dev}} bash -euc 'wasm-pack build --target nodejs --release crates/aozora-wasm --out-dir pkg-nodejs \
        && node crates/aozora-wasm/tests/js/parity.mjs crates/aozora-wasm/pkg-nodejs'

# --- changelog ---------------------------------------------------------------

# CHANGELOG.md is owned by release-plz (`[changelog]` in release-plz.toml): it
# maintains the single root changelog inside the Release PR from the
# Conventional-Commits history. There is no `just changelog` recipe — running
# git-cliff by hand would fight release-plz over the file. To preview the next
# changelog locally, run `release-plz update` (writes Cargo.toml / CHANGELOG.md
# in place; discard the spike with `git restore`).

# --- ci instrumentation (host-only — uses gh CLI auth) ----------------
# `aozora-xtask ci …` is the data-driven CI surface: profile a finished
# workflow run, run every CI job locally before pushing, or replay a
# job through nektos/act. Three reasons these are host-only:
#   - `gh` CLI auth lives on the host (1Password SSH agent etc.).
#   - `act` itself orchestrates Docker; running it inside a Docker dev
#     container means Docker-in-Docker, which is fragile.
#   - The precheck variant *itself* dispatches `docker compose run`, so
#     it must be on the host side of the boundary.
# Skip docker; invoke the binary directly.

# Profile a finished workflow run and rank jobs / steps by wall time.
# Default: latest completed `ci.yml` run on `main`. Pass --run-id to
# pin to a specific run (the value comes from
# `gh run list --branch main --workflow ci`).
ci-profile *ARGS:
    cargo run -q --release -p aozora-xtask -- ci profile {{ARGS}}

# Run every CI job locally and emit a per-job wall-time table.
# Push-time confidence loop. Pass `--list` to see available jobs.
ci-precheck *ARGS:
    cargo run -q --release -p aozora-xtask -- ci precheck {{ARGS}}

# Replay a workflow job through `nektos/act`.
# Heavier than `ci-precheck`; reach for it when the workflow YAML
# itself is the suspect. Requires `act` on PATH (mise can install it
# via `mise use -g github:nektos/act@latest`).
ci-act *ARGS:
    cargo run -q --release -p aozora-xtask -- ci act {{ARGS}}

# Cross-compile aozora-scan to aarch64 + run the proptest suite
# under qemu-user via cross-rs. Verifies the NEON Teddy inner kernel
# matches NaiveScanner byte-identically. Requires `cross` and Docker
# on the host (`cargo install cross` once); mirrors the
# `cross-aarch64` job in ci.yml.
test-aarch64:
    cross test --target aarch64-unknown-linux-gnu -p aozora-scan

# Cross-compile aozora-scan to wasm32-wasip1 to verify the WASM
# SIMD128 kernel codegen. Build-only: `cargo test` is structurally
# impossible on wasm32 because proptest's transitive deps
# (rusty-fork / wait-timeout) require Unix fork() APIs the target
# lacks. The native `cargo nextest run -p aozora-scan` already
# exercises the chunk-level proptests against ScalarTeddyKernel.
# Mirrors the `wasm-test` job in ci.yml; requires `rustup target
# add wasm32-wasip1` once.
test-wasm:
    cargo build --target wasm32-wasip1 -p aozora-scan

# --- aggregate ----------------------------------------------------------------

# Local replica of the full CI pipeline — everything must pass before push.
#
# Order is roughly cheapest-to-most-expensive so a fix-and-retry loop
# fails fast on the early gates. Mirrors every job in ci.yml that does
# not need an external runtime (pandoc, wasm-pack, maturin) which the
# dev image deliberately omits — those three CI-only jobs stay
# unreachable from local.
ci:
    #!/usr/bin/env bash
    set -euo pipefail
    # The two independent gates that don't share cargo's `target/`
    # lock — `cargo deny` / `cargo audit` are metadata-only — run in
    # the background so their wall-time hides behind the cargo chain
    # below instead of stacking on top of it. Verified
    # non-contending against the foreground cargo recipes by manual
    # `just ci` runs; cargo metadata holds an advisory file lock
    # only, never the build write lock.
    just deny           > /tmp/aozora-ci-bg-deny.log 2>&1 &
    deny_pid=$!
    just audit          > /tmp/aozora-ci-bg-audit.log 2>&1 &
    audit_pid=$!

    # Foreground cargo chain in the same cheap-to-expensive order
    # that the original sequential `ci` used, so an early failure
    # still short-circuits before the heavy gates. `lint-full` (not
    # `lint`) so the bench / example targets get the authoritative
    # `clippy-strict` pass, matching CI and the `ci-parallel` gate.
    just lint-full
    just clippy-wasm
    just build
    just drift-gate
    just conformance
    just smoke-ffi
    # Build the Extism plugin to wasm32: this is the only gate that
    # compiles the `#[cfg(target_arch = "wasm32")]` plugin exports (the
    # host build skips them), so it catches plugin-module regressions the
    # `just build` host gate cannot. The heavier `just smoke-extism`
    # (loads the wasm through wasmtime) is left out — its wasmtime compile
    # is too slow for the inline gate; run it manually / in a dedicated CI
    # job.
    just extism-build
    # Go host SDK runtime gate: gofmt + go vet + go test against the
    # freshly-built wasm. CodeQL only *compiles* the Go binding; this is
    # the only gate that runs it, so a host/plugin export-name skew (the
    # SDK calling a function the wasm doesn't export) fails here instead
    # of in a downstream `go get`. Reuses the wasm extism-build produced.
    just smoke-go
    just test
    just test-doc
    just test-doc-all
    just prop
    just shear
    just coverage
    # Playground gates — TypeScript typecheck + vitest unit tests.
    # `docs.yml` workflow runs the same two commands; failing here
    # means failing on CI. Build is intentionally NOT in `just ci`
    # because it triggers wasm-pack again (already covered by
    # `wasm-build`-style host gates) — run `just playground-build`
    # explicitly if needed.
    just playground-typecheck
    just playground-test
    # No-op when AOZORA_CORPUS_ROOT is unset (the recipe prints an
    # informational line and exits 0). On a developer machine that
    # has a corpus checkout exported in the environment, this gives
    # `just ci` an additional adversarial-input pass over real
    # documents — surfacing parse panics / round-trip diverge that
    # the synthetic proptests don't reach.
    just corpus-sweep

    # Reap the background trio. Print their captured output (so
    # failure detail is preserved) and propagate non-zero status.
    failed=0
    if ! wait $deny_pid; then
        echo "::error title=deny::just deny failed (output below)"
        cat /tmp/aozora-ci-bg-deny.log
        failed=1
    fi
    if ! wait $audit_pid; then
        echo "::error title=audit::just audit failed (output below)"
        cat /tmp/aozora-ci-bg-audit.log
        failed=1
    fi
    rm -f /tmp/aozora-ci-bg-{deny,audit}.log
    [[ $failed -eq 0 ]] || exit 1

# Parallel pre-push pipeline — same gates as `ci`, but fast.
#
# The 9-13 min pre-push cost was never the gate logic (warm: the whole
# test suite runs in ~1.3 s); it was redundant recompiles + per-gate
# container starts + strict sequencing. `ci-parallel` removes all three
# without dropping a single gate:
#   1. build + test collapse into `check` (--all-targets compile) +
#      `coverage` (instrumented test build + run + region floor) — one
#      compile instead of three.
#   2. Every gate that does NOT take the container's /cargo/target build
#      lock runs in the BACKGROUND so its wall-time hides behind the
#      foreground cargo chain: deny / audit (metadata)
#      (network), smoke-ffi (host-side target/), playground-typecheck /
#      playground-test (bun), and the non-compiling lint gates
#      fmt-check / typos / strict-code.
#   3. The 4096-case `prop-deep` sweep launches AFTER the foreground
#      `prop` gate (so it reuses the just-built `property_*` binaries —
#      no build-lock contention) and runs in the background, overlapping
#      shear / extism-build / doc instead of adding 3-5 min to the tail.
#
# Foreground stays serial + cheap→expensive so a failure aborts the push
# fast. `SKIP_TAGS=deep just ci-parallel` opts out of prop-deep (the
# narrow escape hatch for an unrelated core regression). CI still runs
# the full matrix on the PR, so it is the authoritative backstop.
#
# `AOZORA_CI_TIMINGS=1 just ci-parallel` prints a per-gate wall-time
# table (slowest first) after a green run — the data-driven way to find
# which gate to optimise (#87). Off by default so the normal push output
# is unchanged; collection is always-on and negligible.
ci-parallel:
    #!/usr/bin/env bash
    set -uo pipefail
    bglog() { echo "/tmp/aozora-cip-$1.log"; }
    bgtime() { echo "/tmp/aozora-cip-$1.time"; }
    declare -A PID
    declare -A DUR
    # `launch` backgrounds a gate; the subshell stamps its own wall time
    # (ms) into a sidecar file so the reap step can fold a concurrent
    # gate's duration into DUR. `run_fg` times a serial gate inline.
    # `print_timings` (opt-in: AOZORA_CI_TIMINGS=1) renders the table.
    launch() {
        local n="$1"; shift
        { __t0=$(date +%s%3N); "$@"; __r=$?; echo "$(( $(date +%s%3N) - __t0 ))" > "$(bgtime "$n")"; exit "$__r"; } > "$(bglog "$n")" 2>&1 &
        PID[$n]=$!
    }
    run_fg() {
        local g="$1"; shift
        echo ":: [fg] $*"
        local t0; t0=$(date +%s%3N)
        "$@"; local rc=$?
        DUR[$g]=$(( $(date +%s%3N) - t0 ))
        return $rc
    }
    print_timings() {
        [[ -n "${AOZORA_CI_TIMINGS:-}" ]] || return 0
        for f in /tmp/aozora-cip-*.time; do
            [[ -e "$f" ]] || continue
            local b; b=$(basename "$f" .time); b=${b#aozora-cip-}
            DUR[$b]=$(cat "$f" 2>/dev/null || echo 0)
        done
        echo ":: ci-parallel per-gate wall time (slowest first):"
        for g in "${!DUR[@]}"; do printf '%s\t%s\n' "${DUR[$g]}" "$g"; done \
            | sort -rn \
            | while IFS=$'\t' read -r ms g; do
                printf '   %4d.%02ds  %s\n' $((ms/1000)) $(((ms%1000)/10)) "$g"
              done
    }

    # Opt-in change-aware fast mode (#81, ADR-0007). DEFAULT is the full
    # gate — the pre-push guarantee ("ローカルで品質完全担保"). When a dev
    # in a hurry sets `AOZORA_CI_FAST=1`, scope the run to gates whose
    # inputs the push range (`origin/main..HEAD`) touched. Safety rails:
    # an unresolvable/empty range, or any change to the gate definitions
    # themselves (infra), forces the full matrix; the cloud CI always runs
    # the full matrix as the backstop, so a too-aggressive skip is caught.
    # (ADR-0007 sketched this as default-on; we inverted it to opt-in so the
    # default push never silently drops coverage — see the ADR follow-up.)
    run_all=1 cats=""
    if [[ -n "${AOZORA_CI_FAST:-}" ]]; then
        base="$(git merge-base origin/main HEAD 2>/dev/null || true)"
        changed=""
        [[ -n "$base" ]] && changed="$(git diff --name-only "$base"..HEAD 2>/dev/null || true)"
        if [[ -n "$changed" ]]; then
            cats="$(printf '%s\n' "$changed" | bash scripts/ci-classify.sh)"
            run_all=0
            [[ " $cats " == *" infra "* ]] && run_all=1
        fi
        if [[ "$run_all" -eq 1 ]]; then
            echo ":: AOZORA_CI_FAST set, but running the FULL matrix (range undeterminable or gate-definition/infra change)."
        else
            echo ":: AOZORA_CI_FAST: change-aware run — touched categories: [${cats:-none}] (cloud CI still runs the full matrix)."
        fi
    fi
    # want <category> — 0 = run this gate, 1 = skip. Full mode runs all;
    want() {
        [[ "$run_all" -eq 1 ]] && return 0
        case "$1" in
            code) [[ " $cats " == *" code "* ]] ;;
            play) [[ " $cats " == *" play "* ]] ;;
            *) return 0 ;;
        esac
    }
    skip() { echo ":: [skip] $1 (AOZORA_CI_FAST: inputs untouched)"; }

    # Background lane — no /cargo/target build-lock contention.
    # verify-spec-vectors is host-side (like smoke-ffi): it drift-checks the
    # vendored spec-vectors/ against the sibling spec repo, a no-op
    # (--allow-missing) where the spec isn't checked out.
    for g in deny audit smoke-ffi verify-spec-vectors; do want code && launch "$g" just "$g"; done
    # fmt-check / typos / strict-code / readme-gate are
    # cheap and apply to any file — always run. ci-fast-selftest guards the
    # change-aware classifier itself (instant host bash).
    for g in fmt-check typos strict-code readme-gate ci-fast-selftest; do launch "$g" just "$g"; done
    # playground-typecheck + playground-test share one `node_modules`
    # volume; launching them as two concurrent gates makes their
    # `_playground-ensure` (`bun install`) hard-link into that volume in
    # parallel and intermittently fail with `EEXIST`. Run both through one
    # sequential job so the install happens exactly once, single-threaded.
    if want play; then launch playground-ci just playground-ci; else skip playground-ci; fi

    # Foreground cargo chain — serial (shared build lock), fail-fast.
    # Lint runs the AUTHORITATIVE surface, not the lighter per-commit
    # `clippy`: `clippy-strict` is `--all-targets` (examples + benches,
    # aozora-bench included) and `clippy-wasm` lints the wasm32-only
    # binding modules — exactly the two cells (`lint (clippy-strict)` +
    # `wasm-build`'s clippy step) that GitHub runs. Keeping them here
    # means a doc_markdown / missing_docs slip in a bench example or a
    # wasm32 cfg module is caught BEFORE the push, not by CI. CI is the
    # insurance, the pre-push gate is the guarantee.
    fg_failed=""
    # `test-internals` runs RIGHT AFTER `coverage`: coverage executes the
    # default-feature suite (and measures regions); test-internals then runs
    # aozora-lsp's `internals`-gated integration suites that coverage skips.
    for gate in clippy-strict clippy-wasm check drift-gate conformance coverage test-internals prop; do
        want code || { skip "$gate"; continue; }
        if ! run_fg "$gate" just "$gate"; then fg_failed="$gate"; break; fi
    done

    # property_* binaries are now built → deep sweep reuses them (no
    # rebuild) and overlaps the remaining foreground gates.
    if [[ -z "$fg_failed" ]]; then
        if want code && [[ "${SKIP_TAGS:-}" != *deep* ]]; then
            launch prop-deep just prop-deep
        else
            echo ":: prop-deep skipped (SKIP_TAGS=deep or AOZORA_CI_FAST: no code change)"
        fi
        for gate in shear test-doc test-doc-all extism-build parity-wasm smoke-go doc corpus-sweep; do
            case "$gate" in
                *) want code || { skip "$gate"; continue; } ;;
            esac
            if ! run_fg "$gate" just "$gate"; then fg_failed="$gate"; break; fi
        done
    fi

    # Fail fast on a foreground failure (background gates self-clean via --rm).
    if [[ -n "$fg_failed" ]]; then
        echo "::error title=${fg_failed}::foreground gate failed — re-run \`just ${fg_failed}\` for the unwrapped output."
        exit 1
    fi

    # Foreground passed → reap the background lane.
    failed=0
    for name in "${!PID[@]}"; do
        if ! wait "${PID[$name]}"; then
            echo "::error title=${name}::background gate failed (log below)"
            cat "$(bglog "$name")" 2>/dev/null || true
            failed=1
        fi
    done
    print_timings
    rm -f /tmp/aozora-cip-*.log /tmp/aozora-cip-*.time
    [[ $failed -eq 0 ]] || { echo "ci-parallel: a background gate failed (see above)"; exit 1; }
    echo "ci-parallel: all gates passed ✔"

# Self-check for the change-aware classifier (`scripts/ci-classify.sh`)
# that `AOZORA_CI_FAST=1 just ci-parallel` relies on. Asserts the category
# set for representative diffs so a misclassification can never silently
# widen a skip. Pure host bash; runs in `ci-parallel`'s always-on lane.
ci-fast-selftest:
    #!/usr/bin/env bash
    set -euo pipefail
    fail=0
    check() {
        local label="$1" expected="$2"; shift 2
        local got; got="$(printf '%s\n' "$@" | bash scripts/ci-classify.sh)"
        if [[ "$got" == "$expected" ]]; then
            echo "ok   $label → [$got]"
        else
            echo "FAIL $label → got [$got] want [$expected]"; fail=1
        fi
    }
    check "rust source"        "code"       "crates/aozora-syntax/src/format.rs"
    check "rust doc comment"   "code"       "crates/aozora/src/document.rs"
    check "Cargo manifest"     "code"       "crates/aozora/Cargo.toml"
    check "conformance vector" "code"       "crates/aozora-conformance/spec-vectors/x.json"
    check "playground only"    "play"       "playground/src/App.tsx"
    check "ADR / docs"         "book"       "docs/adr/0017-x.md"
    check "root README"        "code"       "README.md"
    check "Justfile = infra"   "infra"      "Justfile"
    check "workflow = infra"   "infra"      ".github/workflows/ci.yml"
    check "scripts = infra"    "infra"      "scripts/ci-classify.sh"
    check "docs + code"        "code book"  "crates/aozora/src/document.rs" "docs/adr/0017-x.md"
    check "play + docs"        "play book"  "playground/src/App.tsx" "docs/x.md"
    check "infra forces full"  "code infra" "crates/x/src/a.rs" "lefthook.yml"
    if [[ $fail -eq 0 ]]; then
        echo "ci-fast-selftest: all classifications correct ✔"
    else
        echo "ci-fast-selftest: classifier regression — fix scripts/ci-classify.sh"; exit 1
    fi

# --- developer workflow helpers ----------------------------------------------

# Run after a build to verify the cache is actually warm; a first-hand
# way to notice when `RUSTC_WRAPPER` gets defeated by stray env or profile tweaks.
# Show sccache hit/miss ratio, cache size, fetch counts.
sccache-stats:
    {{_dev}} sccache --show-stats

# Reset sccache counters to zero.
# Useful before a measurement window:
#   just sccache-zero && just clean && just build && just sccache-stats
sccache-zero:
    {{_dev}} sccache --zero-stats

# Start the bacon file-watcher inside the dev container.
# Defaults to the `check` job; pass a job name to pick another, e.g.
# `just watch clippy`. Keybindings: `t` test / `c` clippy / `d` doc /
# `f` failing-only / `esc` previous job / `q` quit / Ctrl-J list jobs.
watch JOB="":
    {{_dev}} bacon {{JOB}}

# Watch + re-run clippy on every save (bacon `clippy` job — same lint
# surface as `just clippy`). Fast incremental lint feedback.
watch-lint:
    {{_dev}} bacon clippy

# Watch + re-run the nextest suite on every save (bacon `test` job).
watch-test:
    {{_dev}} bacon test

# Headless bacon run (no TUI).
# Keeps the watch loop but prints plain lines. Useful for piping output
# (`| tee`) and for sessions without a TTY.
watch-headless JOB="check":
    {{_ci}} bacon --headless --job {{JOB}}

# Install git hooks (pre-commit / commit-msg / pre-push).
# Idempotent — re-run safely after lefthook.yml edits or to repair stubs.
hooks:
    {{_dev}} lefthook install

# --- playground (Solid + Vite + WASM frontend) -------------------------------
#
# The playground lives under `playground/` and is its own Bun project,
# but every gate runs through the dev / playground containers — never
# directly on the host — so contributors don't need to install bun /
# wasm-pack locally. `_pg` points at the `playground` service whose
# working_dir is already `/workspace/playground` and whose
# `node_modules` / `dist` live in named volumes (no host bleed).

_pg := "docker compose run --rm --no-TTY playground"
# Same service but privileged (root), for the one-off ownership fix below.
_pg_root := "docker compose run --rm --no-TTY --user 0 playground"

# Build the WASM `pkg/` that `vite.config.ts`'s alias targets. Must run
# before `playground-build` (when `.d.ts` is missing or stale).
playground-wasm:
    {{_dev}} wasm-pack build --target web --release crates/aozora-wasm

# Normalise the playground's `node_modules` / `dist` named-volume trees to
# the compose runtime UID/GID. Docker creates a named volume root-owned on
# first mount, and a build that once ran as root (e.g. an old CI run over the
# same volume) leaves root-owned files behind, so the service — which runs as
# the host UID (compose `user:`) — then fails `vite build` (which empties
# dist) and bun's writes with EACCES. A one-off privileged (`--user 0`)
# container fixes ownership. Guard on `find ! -uid` (not just the top dir:
# the root can be correct while a child is stale) so a full `chown -R` runs
# only when some entry is wrongly owned — a cheap early-exit scan when clean, so
# it is safe to depend on from `_playground-ensure` on every run. (In CI,
# AOZORA_UID=0 → the volumes match the root runtime and this no-ops.)
_playground-fix-perms:
    {{_pg_root}} sh -euc 'u={{ env_var_or_default("AOZORA_UID", "1000") }}; g={{ env_var_or_default("AOZORA_GID", "1000") }}; for d in node_modules dist; do [ -d "$d" ] || continue; if [ -n "$(find "$d" ! -uid "$u" -print -quit)" ]; then chown -R "$u:$g" "$d"; fi; done'

# Ensure the playground's prerequisites exist before typecheck / test:
# the wasm `pkg/` that tsc + vite alias `aozora-wasm` to, and the bun
# `node_modules`. `pkg/` persists in the tree and `node_modules` in a
# named volume, so on a warm checkout the wasm build is skipped and
# `bun install` is a fast lockfile check. A FRESH checkout now
# self-initialises here instead of failing `just ci` with
# "cannot find module 'aozora-wasm'" / "tsc: command not found".
_playground-ensure: _playground-fix-perms
    [ -d crates/aozora-wasm/pkg ] || just playground-wasm
    {{_pg}} bun install

# Type-check playground TypeScript sources.
playground-typecheck: _playground-ensure
    {{_pg}} bun run typecheck

# Run vitest unit tests for the playground
# (share / storage / parserState / utils — see src/__tests__/).
playground-test: _playground-ensure
    {{_pg}} bun run test

# Combined playground gate for `ci-parallel`: ensure deps once, then
# typecheck + test + CSS lint in a single sequential job. `ci-parallel` runs
# its gates concurrently; if `playground-typecheck` and `playground-test`
# launched separately, their `_playground-ensure` (`bun install`) would
# hard-link into the shared `node_modules` volume in parallel and hit
# `Failed to link …: EEXIST`. One job keeps the install single-threaded.
# `lint:css` runs stylelint over the playground CSS *and* the canonical
# aozora-notation.css (visible in-container via the full-repo mount); this
# single chokepoint wires it into both pre-push (ci-parallel) and CI
# (playground-checks) with no ci.yml change.
# Standalone `playground-typecheck` / `playground-test` are unchanged.
playground-ci: _playground-ensure
    {{_pg}} bun run typecheck
    {{_pg}} bun run test
    {{_pg}} bun run lint:css

# Production build of the playground. Regenerates the WASM bundle
# first so the vite alias target is always fresh; `_playground-ensure`
# then guarantees bun deps and correct volume ownership so `vite build`
# can empty `dist`.
playground-build: playground-wasm _playground-ensure
    {{_pg}} bun run build

# All playground gates in one shot — typecheck + test + build.
playground-all: playground-typecheck playground-test playground-build

# Playwright E2E smoke suite (#335 D-5). Runs in the dev-image `playground`
# service (bun + Rust present): `playground-wasm` rebuilds the WASM engine
# fresh (the E2E exercises runtime *rendering*, so a stale `pkg/` — which
# `_playground-ensure`'s "build only if absent" guard would happily keep —
# must not be trusted; this mirrors the CI e2e job, which always builds wasm),
# `_playground-ensure` adds bun deps, then chromium is installed and Playwright
# drives a prod `vite preview` build (its `webServer`). Best-effort locally — chromium's
# system libraries are not in the dev image, so a local run may fail at browser
# launch; the CI `e2e` job (host runner, `playwright install --with-deps`) is
# the authoritative gate. Deliberately NOT wired into `ci-parallel`: a second
# concurrent bun-install lane against the shared node_modules volume would
# re-introduce the `EEXIST` race.
playground-e2e: playground-wasm _playground-ensure
    # One root container does browser-install + test together: `--with-deps`
    # apt-installs chromium's system libraries (libnspr4 etc., absent from the
    # dev image) which needs root, and a single `docker compose run` keeps the
    # browser cache (unmounted `~/.cache/ms-playwright`) and the test run in the
    # same ephemeral container. (CI runs the equivalent as separate steps on one
    # host runner, where the filesystem is shared and sudo is available.)
    docker compose run --rm --no-TTY --user root playground \
        bash -c "bun x playwright install --with-deps chromium && bun x playwright test"

# --- VS Code extension (TypeScript, esbuild-bundled) --------------------------
#
# The extension lives under `editors/vscode/` and is its own Bun project.
# `vscode-ci` mirrors the CI `vscode` job: biome lint + tsc typecheck
# (`check`), the esbuild bundle (`compile`, which inlines the renderer's
# canonical stylesheet — ADR-0024), and the `node --test` security suite. It
# runs in the dev image (bun present) over the bind-mounted checkout. Like
# `playground-e2e`, it's a bun gate kept out of the change-aware `ci-parallel`;
# the CI `vscode` job (host runner) is the authoritative gate — run this for a
# quick local check.
vscode-ci:
    {{_dev}} bash -euc 'cd editors/vscode && bun install --frozen-lockfile && bun run check && bun run compile && bun run test'

# --- profiling (samply, host-only) -------------------------------------------
# samply uses perf_event_open(2) which Docker's seccomp profile blocks; the
# xtask binary therefore runs on the host (not via {{_dev}}). Requires
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
    {{_dev}} lefthook uninstall

# --- cleanup ------------------------------------------------------------------

# Remove build artifacts (keeps volumes; use `docker compose down -v` for volumes)
clean:
    {{_dev}} cargo clean --workspace

# Tear down all compose state (destroys cached registry/target/sccache volumes)
nuke:
    docker compose down -v --remove-orphans
