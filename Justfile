# aozora workspace task runner.
# The ONE entry point for every development operation. Every target runs inside Docker;
# never invoke cargo, mdbook, or playwright on the host directly.

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

# Build all workspace crates
build:
    {{_dev}} cargo build --workspace --all-targets

# Build release binaries
build-release:
    {{_dev}} cargo build --release --workspace

# Drop into an interactive dev shell
shell:
    {{_dev}} bash

# Run the aozora CLI with arbitrary args (`just run check FILE`, etc.)
run *ARGS:
    {{_dev}} cargo run --package aozora-cli --quiet -- {{ARGS}}

# --- tests --------------------------------------------------------------------

# Run the full test suite (unit + integration + snapshot)
test *ARGS:
    {{_dev}} cargo nextest run --workspace --all-targets {{ARGS}}

# Run doctests (nextest skips these by design)
test-doc:
    {{_dev}} cargo test --workspace --doc

# Property-based tests only. Default 128 cases per proptest block
# (AOZORA_PROPTEST_CASES override via aozora-test-utils::config). Fast
# enough to live in `just ci` — see `just prop-deep` for a stress run.
prop:
    {{_dev}} cargo nextest run --workspace --all-features --test 'property_*' --run-ignored default

# Deep property sweep — 4096 cases per block, used before cutting a
# release to exercise invariants beyond the default CI budget.
prop-deep:
    {{_dev}} bash -c 'AOZORA_PROPTEST_CASES=4096 cargo nextest run --workspace --all-features --test "property_*" --run-ignored default'

# Unit-test-only predicate pinning — runs every `invariant_unit_` test
# in `aozora_parser::test_support`. Narrow target for regression hunts
# that don't need the full proptest sweep.
invariants:
    {{_dev}} cargo nextest run --package aozora-parser --lib -E 'test(invariant_unit_)'

# Aozora annotation fixtures (hand-written, ~40 cases)
spec-aozora:
    {{_dev}} cargo nextest run --package aozora-parser --test aozora_spec

# Golden fixture: 罪と罰 (card 56656) — Tier-A acceptance gate
# (panic-free + zero unconsumed ［＃ markers in the rendered HTML).
spec-golden-56656:
    {{_dev}} cargo nextest run --package aozora-parser --test golden_56656

# Property-based sweep over whatever directory `AOZORA_CORPUS_ROOT` points at.
# Bind-mounts the corpus dir into the container at a stable path so the
# test binary reads it from the same location regardless of the host path.
# Runtime-skips with an informational message if the env var is unset —
# this is *not* a failure, just an indication that no corpus is configured.
#
# Usage:
#   export AOZORA_CORPUS_ROOT=$HOME/aozora-corpus
#   just corpus-sweep
#
# Invariants checked (report/enforcement split documented in the test
# itself at crates/aozora-parser/tests/corpus_sweep.rs, and in ADR-0005):
#   I1 — no panic on any input (hard).
#   I2 — no unconsumed ［＃ markers (hard).
#   I3 — serialize ∘ parse fixed point (hard).
#   I4 — emitted HTML is tag-balanced (hard).
#   I5 — SJIS decode stable (report-only).
#   I6 — no PUA sentinel U+E001–U+E004 in HTML (hard, budget=0).
#   I7 — every afm-* class is in AFM_CLASSES (hard, budget=0).
#   I8 — no <script / javascript: / on<event>= markers (hard, budget=0).
#   I9 — afm-annotation wrapper shape is well-formed (hard, budget=0).
#   I10 — no afm-indent / afm-annotation inside <h1>-<h6> (hard, budget=0).
# Per-invariant budget overrides via AOZORA_CORPUS_I{2,3,4,6,7,8,9,10}_BUDGET.
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
        dev cargo nextest run --package aozora-parser --test corpus_sweep --no-capture

# Fuzz smoke (60s per harness) — runs the registered cargo-fuzz harnesses
fuzz *ARGS:
    {{_dev}} bash -c 'cd crates/aozora-parser && cargo +nightly fuzz run {{ARGS}}'

# Benchmarks (criterion)
bench *ARGS:
    {{_dev}} cargo bench --workspace {{ARGS}}

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
#   over their crate libraries; wiring integration tests against the
#   process entry is follow-up work.
#
# `_COV_FLOOR` is the enforced minimum today, not the goal. The
# stated goal (ADR-0004 §coverage) is 100% on production code. The
# floor ratchets upward in follow-up commits that close specific gaps.
_COV_FLOOR := "0"
_COV_IGNORE := "(target/|/main\\.rs$)"

coverage:
    {{_dev}} cargo llvm-cov nextest \
        --workspace \
        --ignore-filename-regex '{{_COV_IGNORE}}' \
        --fail-under-regions {{_COV_FLOOR}}

# HTML coverage report for local inspection. No threshold — intended
# for opening `coverage/html/index.html` in a browser.
coverage-html:
    {{_dev}} cargo llvm-cov nextest \
        --workspace \
        --ignore-filename-regex '{{_COV_IGNORE}}' \
        --html --output-dir coverage/html

# Branch-level coverage report (requires nightly for `--branch` support).
# Informational only — no threshold. Use to surface uncovered conditionals
# when working a specific file toward C1 100%.
coverage-branch:
    {{_dev}} cargo +nightly llvm-cov nextest \
        --branch \
        --workspace \
        --ignore-filename-regex '{{_COV_IGNORE}}'

# --- lint / static analysis ---------------------------------------------------

# Run all lints (fmt + clippy + typos + strict-code)
lint: fmt-check clippy typos strict-code

# Forbid patterns that hide bugs or introduce unstable/unsafe surface in our
# own crates. Every check is defensive — each represents a pattern we have
# decided IS a bug-source and want rejected at the gate rather than fought
# later in code review.
strict-code:
    #!/usr/bin/env bash
    set -euo pipefail
    shopt -s globstar
    files=(crates/**/*.rs)

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
    check 'warning suppression (#[allow] / cfg_attr+allow)' \
        '^\s*(#!?\[allow\(|#!?\[cfg_attr\([^)]*allow\()' || failed=1

    # ---- Nightly / unstable feature gates ----------------------------------
    check 'nightly feature gate (#[feature] / #![feature])' \
        '^\s*#!?\[feature\(' || failed=1

    # ---- Unsafe code -------------------------------------------------------
    # Every crate root has `#![forbid(unsafe_code)]` (checked below); this
    # text-level grep is belt-and-braces for typos that would defeat the
    # compiler gate.
    check 'unsafe code (unsafe fn / unsafe { / unsafe impl / unsafe trait)' \
        '(^|[^a-zA-Z_#])unsafe\s+(fn|impl|trait|\{)' || failed=1

    # ---- Required deny directive -------------------------------------------
    for root in crates/*/src/lib.rs crates/*/src/main.rs; do
        [[ -f "$root" ]] || continue
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
        | grep -vE '(#[0-9]+|M[0-9]|issue|ADR-[0-9]+)' || true)
    if [[ -n "$todo_hits" ]]; then
        echo '==> forbidden: bare TODO/FIXME/XXX without an issue or milestone reference' >&2
        echo "$todo_hits" >&2
        failed=1
    fi

    # ---- println! / eprintln! in library crates ----------------------------
    # Library crates emit observability via `tracing`, not raw print.
    # CLI crates (aozora-cli) and tests/examples/fuzz are exempt.
    lib_files=(crates/aozora-syntax/**/*.rs crates/aozora-lexer/**/*.rs crates/aozora-parser/**/*.rs crates/aozora-encoding/**/*.rs)
    print_hits=$(grep -nE '(^|[^[:alnum:]_])e?print(ln)?!\s*\(' "${lib_files[@]}" 2>/dev/null \
        | grep -vE '/(tests|benches|examples|fuzz_targets)/' || true)
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

    if [[ $failed -ne 0 ]]; then
        echo "" >&2
        echo "strict-code check failed. Refactor the offending sites; do not silence." >&2
        exit 1
    fi
    echo "strict-code: clean"

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
clippy:
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

# Unused dependency scan (requires nightly)
udeps:
    {{_dev}} cargo +nightly udeps --workspace --all-targets

# Semver break detection (runs against published baseline once crates are on crates.io)
semver:
    {{_dev}} cargo semver-checks check-release --workspace

# --- dependency follow-up (local-only, no remote CI) -------------------------
# Policy: workspace deps track @latest. The mechanism is purely local —
# `just deps-check` runs the full dependency-health gate (outdated +
# audit + deny), `just upgrade` bumps Cargo.toml to the latest
# compatible versions, and a systemd user timer (see
# `scripts/install-deps-timer.sh`) runs `just deps-check` weekly so
# new advisories surface even on quiet branches. ADR-0018 records
# the policy + cadence rationale.

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

# Full dependency-health gate: outdated + audit + deny + udeps. Marks
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
# harness against it, runs end-to-end. The 11-check harness exercises
# every public C entry point on the happy path plus three error
# cases (null input, invalid UTF-8, PUA collision).
smoke-ffi:
    bash crates/aozora-ffi/tests/c_smoke/run.sh

# --- corpus / spec helpers ---------------------------------------------------

# New Architecture Decision Record (MADR template)
adr TITLE:
    {{_dev}} cargo run --package xtask --quiet -- new-adr {{TITLE}}

# Refresh the Aozora corpus lockfile (re-pins works by current SHA256)
corpus-refresh:
    {{_dev}} cargo run --package xtask --quiet -- corpus-refresh

# Regenerate CHANGELOG.md from Conventional-Commits history (see cliff.toml).
changelog:
    {{_dev}} git-cliff -o CHANGELOG.md

# --- aggregate ----------------------------------------------------------------

# Local replica of the full CI pipeline — everything must pass before push
ci:
    just lint
    just build
    just test
    just prop
    just spec-aozora
    just spec-golden-56656
    just deny
    just audit
    just udeps
    just coverage

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
