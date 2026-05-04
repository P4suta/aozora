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

# --- build/shell --------------------------------------------------------------

# Build all workspace crates.
#
# `aozora-bench` is excluded from every workspace-wide CI gate
# (build / test / coverage / clippy / udeps) because it's a
# bench-only harness whose dep tree pulls in `zstd-sys`,
# `criterion`, `addr2line`, `gimli`, `object`, and `ruzstd` —
# adding ~100 s of cold-cache compile time that no other crate in
# the workspace needs. Bench runs go through `just bench`, which
# explicitly invokes `cargo bench --workspace` and gets the full
# tree on demand.
build:
    {{_dev}} cargo build --workspace --exclude aozora-bench --all-targets

# Build release binaries
build-release:
    {{_dev}} cargo build --release --workspace --exclude aozora-bench

# Drop into an interactive dev shell
shell:
    {{_dev}} bash

# Run the aozora CLI with arbitrary args (`just run check FILE`, etc.)
run *ARGS:
    {{_dev}} cargo run --package aozora-cli --quiet -- {{ARGS}}

# --- tests --------------------------------------------------------------------

# Run the full test suite (unit + integration + snapshot).
# `aozora-bench` is excluded — see `build` above for rationale.
test *ARGS:
    {{_dev}} cargo nextest run --workspace --exclude aozora-bench --all-targets {{ARGS}}

# Run doctests (nextest skips these by design)
test-doc:
    {{_dev}} cargo test --workspace --doc

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
# crates/aozora-book/src/wire/. Run after touching any wire struct
# or `aozora::wire::SCHEMA_VERSION`; commit the resulting diff so
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

# Phase L4 — bundled drift gate. Equivalent to the CI `drift-gate`
# job: schema + types in one shot. Use locally before pushing.
#
# Inlined as a single `docker compose run` rather than a recipe-deps
# chain (`drift-gate: schema-check types-check`) so both checks share
# one container start. The previous form burned a full container
# bootstrap (rustup channel sync + components download, ~22 s) twice
# per CI invocation; the bash -c form runs the second xtask invocation
# against an already-warm container with the xtask binary cached in
# `target/`.
drift-gate:
    {{_dev}} bash -c 'set -euo pipefail; cargo run -p aozora-xtask -q -- schema check && cargo run -p aozora-xtask -q -- types check'

# Phase O4 — WPT-style conformance runner. Walks every fixture
# under aozora-conformance/fixtures/render/, runs the parser, and
# fails non-zero if any `must`-tier case regresses. Writes a
# per-case results.json into the handbook source tree so readers
# can see the latest tier breakdown.
conformance:
    {{_dev}} cargo run -p aozora-xtask -q -- conformance run

# Property-based tests only. Default 128 cases per proptest block
# (AOZORA_PROPTEST_CASES override via aozora-test-utils::config). Fast
# enough to live in `just ci` — see `just prop-deep` for a stress run.
prop:
    {{_dev}} cargo nextest run --workspace --all-features --test 'property_*' --run-ignored default

# Deep property sweep — 4096 cases per block, used before cutting a
# release to exercise invariants beyond the default CI budget.
prop-deep:
    {{_dev}} bash -c 'AOZORA_PROPTEST_CASES=4096 cargo nextest run --workspace --all-features --test "property_*" --run-ignored default'

# Walk every document under `AOZORA_CORPUS_ROOT` and check parse +
# round-trip invariants on the public `aozora::Document` surface.
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
        dev cargo nextest run --package aozora --test corpus_sweep --no-capture

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
# in follow-up commits that close specific gaps. Today's measurement
# (73.93%) sets the floor at 73 with a 1-point margin so a borderline
# refactor doesn't trip the gate spuriously — push it up by hand
# whenever a coverage-closing PR lands.
_COV_FLOOR := "73"
_COV_IGNORE := "(target/|/main\\.rs$)"

coverage:
    {{_dev}} cargo llvm-cov nextest \
        --workspace --exclude aozora-bench \
        --ignore-filename-regex '{{_COV_IGNORE}}' \
        --fail-under-regions {{_COV_FLOOR}}

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

# --- lint / static analysis ---------------------------------------------------

# Run all lints (fmt + clippy + typos + strict-code + doc)
lint: fmt-check clippy typos strict-code doc

# Build rustdoc with `-D warnings`. Mirrors the `docs` workflow's
# `Build rustdoc` step so a doc-link or rustdoc-lint regression fails
# locally before it reaches the Pages deploy. Stays scoped to the
# workspace lint config (`broken_intra_doc_links = "deny"` in
# `[workspace.lints.rustdoc]` plus the `RUSTDOCFLAGS` env to lift the
# remaining warn-level lints to errors).
doc:
    {{_dev}} env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items

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
    #   - aozora-scan  : x86_64 AVX2 intrinsics (SIMD scanner)
    #   - aozora-xtask : dev-tooling binary; `#[allow(reason=...)]`
    #                    for narrow clippy carve-outs is acceptable
    #                    here per Rust 1.81+ stable convention
    #
    # The grep below skips these paths; everything else stays under the
    # universal "no unsafe" gate.
    is_unsafe_exempt() {
        case "$1" in
            crates/aozora-ffi/*|crates/aozora-scan/*|crates/aozora-xtask/*) return 0 ;;
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
    expect_files=(crates/aozora-pipeline/src/**/*.rs)
    expect_count=$(grep -hcE '\.expect\(' "${expect_files[@]}" 2>/dev/null \
        | awk '{s+=$1} END {print s+0}')
    expect_baseline=50
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

# Format check (no-write)
fmt-check:
    {{_dev}} cargo fmt --all -- --check

# Auto-format (writes)
fmt:
    {{_dev}} cargo fmt --all

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
# + benches), and the bench crate is no longer excluded. Used by
# lefthook pre-commit so the bench / example targets that the CI
# `clippy` recipe skips still get a lint pass before the commit
# lands. Slower per-commit, but the matrix-split CI lint job is
# correspondingly leaner.
clippy-strict:
    {{_dev}} cargo clippy --workspace --all-targets --all-features -- -D warnings

# Typo check
typos:
    {{_dev}} typos

# Dependency linting (licenses, advisories, bans)
deny:
    {{_dev}} cargo deny check

# RustSec advisory scan
audit:
    {{_dev}} cargo audit

# Unused dependency scan (requires nightly).
# `aozora-bench` is excluded for the same reason build / test / clippy
# exclude it (heavy bench dep tree). The bench crate gets its own
# udeps run through `just udeps-bench` when needed.
udeps:
    {{_dev}} cargo +nightly udeps --workspace --exclude aozora-bench --all-targets

# Unused-dep scan limited to the bench crate. Run before cutting a
# release if `aozora-bench` had dep changes; not part of the per-PR
# CI gate (cf. `just udeps`).
udeps-bench:
    {{_dev}} cargo +nightly udeps -p aozora-bench --all-targets

# Semver break detection (runs against published baseline once crates are on crates.io)
semver:
    {{_dev}} cargo semver-checks check-release --workspace

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

# --- changelog ---------------------------------------------------------------

# Regenerate CHANGELOG.md from Conventional-Commits history (see cliff.toml).
# `--unreleased` keeps the file pinned to a Keep-a-Changelog "Unreleased"
# section between tags; the `release.yml` pipeline replaces it with the
# tagged release notes at version-cut time.
changelog:
    {{_dev}} git-cliff --unreleased --prepend CHANGELOG.md

# Regenerate CHANGELOG.md from scratch (full history). Rarely needed —
# the in-place `changelog` recipe is the canonical update path.
changelog-full:
    {{_dev}} git-cliff -o CHANGELOG.md

# --- mdbook handbook ---------------------------------------------------------
# `crates/aozora-book` is rendered by mdbook with the `mdbook-mermaid`
# preprocessor (architecture pipeline / arena lifetime diagrams). Link
# verification uses `lychee` rather than `mdbook-linkcheck`, because the
# latter chronically lags upstream mdbook's RenderContext schema.
_book := "docker compose run --rm book"

# Build the handbook into crates/aozora-book/book/.
book-build:
    {{_book}} mdbook build

# Live-preview at http://localhost:3000. Re-renders on every save.
book-serve:
    docker compose up book

# Crawl every internal + external link in the rendered handbook.
# Run after `book-build`; lychee uses the generated HTML, not the source
# Markdown, so cross-page anchors are validated post-render.
# Concurrency / retries / 404-skip / accept policy live in
# `crates/aozora-book/lychee.toml` so the same config applies to
# `just book-linkcheck` and the `book` CI job.
book-linkcheck:
    {{_book}} mdbook build
    {{_book}} lychee --config lychee.toml 'book/**/*.html'

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

# --- aggregate ----------------------------------------------------------------

# Local replica of the full CI pipeline — everything must pass before push.
#
# Order is roughly cheapest-to-most-expensive so a fix-and-retry loop
# fails fast on the early gates. Mirrors every job in ci.yml that does
# not need an external runtime (pandoc, wasm-pack, maturin) which the
# dev image deliberately omits — those three CI-only jobs stay
# unreachable from local.
#
# `book-linkcheck` ends the chain because lychee's network probes are
# the slowest gate and the only one that depends on external availability;
# putting it last means a transient pyo3.rs / docs.rs hiccup doesn't
# delay the deterministic gates' failure signal.
ci:
    #!/usr/bin/env bash
    set -euo pipefail
    # The three independent gates that don't share cargo's `target/`
    # lock — `cargo deny` / `cargo audit` are metadata-only and
    # `book-linkcheck` is a lychee network probe — run in the
    # background so their wall-time hides behind the cargo chain
    # below instead of stacking on top of it. Verified
    # non-contending against the foreground cargo recipes by manual
    # `just ci` runs; cargo metadata holds an advisory file lock
    # only, never the build write lock.
    just deny           > /tmp/aozora-ci-bg-deny.log 2>&1 &
    deny_pid=$!
    just audit          > /tmp/aozora-ci-bg-audit.log 2>&1 &
    audit_pid=$!
    just book-linkcheck > /tmp/aozora-ci-bg-book.log 2>&1 &
    book_pid=$!

    # Foreground cargo chain in the same cheap-to-expensive order
    # that the original sequential `ci` used, so an early failure
    # still short-circuits before the heavy gates.
    just lint
    just build
    just drift-gate
    just conformance
    just smoke-ffi
    just test
    just prop
    just udeps
    just coverage
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
    if ! wait $book_pid; then
        echo "::error title=book-linkcheck::just book-linkcheck failed (output below)"
        cat /tmp/aozora-ci-bg-book.log
        failed=1
    fi
    rm -f /tmp/aozora-ci-bg-{deny,audit,book}.log
    [[ $failed -eq 0 ]] || exit 1

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

# Headless bacon run (no TUI).
# Keeps the watch loop but prints plain lines. Useful for piping output
# (`| tee`) and for sessions without a TTY.
watch-headless JOB="check":
    {{_ci}} bacon --headless --job {{JOB}}

# Install git hooks (pre-commit / commit-msg / pre-push).
# Idempotent — re-run safely after lefthook.yml edits or to repair stubs.
hooks:
    {{_dev}} lefthook install

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
