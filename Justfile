# afm workspace task runner.
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

# Run the afm CLI with arbitrary args (same as ./bin/afm ARGS)
run *ARGS:
    {{_dev}} cargo run --package afm-cli --quiet -- {{ARGS}}

# --- tests --------------------------------------------------------------------

# Run the full test suite (unit + integration + snapshot)
test *ARGS:
    {{_dev}} cargo nextest run --workspace --all-targets {{ARGS}}

# Run doctests (nextest skips these by design)
test-doc:
    {{_dev}} cargo test --workspace --doc

# Property-based tests only
prop:
    {{_dev}} cargo nextest run --workspace --all-features --test 'property_*' --run-ignored default

# CommonMark 0.31.2 spec compliance (652 cases, pass = 652/652)
spec-commonmark:
    {{_dev}} cargo nextest run --package afm-parser --test commonmark_spec

# GitHub Flavored Markdown spec compliance
spec-gfm:
    {{_dev}} cargo nextest run --package afm-parser --test gfm_spec

# Aozora annotation fixtures (hand-written, ~40 cases)
spec-aozora:
    {{_dev}} cargo nextest run --package afm-parser --test aozora_spec

# Golden fixture: 罪と罰 (card 56656), the M0 acceptance test
spec-golden-56656:
    {{_dev}} cargo nextest run --package afm-parser --test golden_56656

# 120-work Aozora regression corpus (Tier A + B are hard gates)
corpus *ARGS:
    {{_dev}} cargo run --package xtask --quiet -- corpus-test {{ARGS}}

# Property-based sweep over whatever directory `AFM_CORPUS_ROOT` points at.
# Bind-mounts the corpus dir into the container at a stable path so the
# test binary reads it from the same location regardless of the host path.
# Runtime-skips with an informational message if the env var is unset —
# this is *not* a failure, just an indication that no corpus is configured.
#
# Usage:
#   export AFM_CORPUS_ROOT=$HOME/aozora-corpus
#   just corpus-sweep
#
# Invariants checked (report/enforcement split documented in the test
# itself at crates/afm-parser/tests/corpus_sweep.rs):
#   I1 — no panic on any input (hard).
#   I2 — no unconsumed ［＃ markers (report-only, M1 D pending).
#   I5 — SJIS decode stable (report-only).
corpus-sweep:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${AFM_CORPUS_ROOT:-}" ]]; then
        echo "AFM_CORPUS_ROOT is not set; sweep has nothing to walk."
        echo "Set it to a directory of aozora-format .txt files, e.g.:"
        echo "  export AFM_CORPUS_ROOT=\$HOME/aozora-corpus"
        echo "Then re-run 'just corpus-sweep'."
        exit 0
    fi
    if [[ ! -d "$AFM_CORPUS_ROOT" ]]; then
        echo "AFM_CORPUS_ROOT=$AFM_CORPUS_ROOT is not a directory." >&2
        exit 1
    fi
    docker compose run --rm \
        -v "$AFM_CORPUS_ROOT":/corpus:ro \
        -e AFM_CORPUS_ROOT=/corpus \
        dev cargo nextest run --package afm-parser --test corpus_sweep --no-capture

# Fuzz smoke (60s per harness) — runs the registered cargo-fuzz harnesses
fuzz *ARGS:
    {{_dev}} bash -c 'cd crates/afm-parser && cargo +nightly fuzz run {{ARGS}}'

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
# - `upstream/comrak/` — vendored fork (ADR-0001), never measured here.
# - `target/` — build artefacts.
# - `**/main.rs` — CLI binary entrypoints (`afm-cli`, `xtask`). These
#   are thin shells over their crate libraries; wiring integration
#   tests against the process entry is follow-up work.
# - `xtask/` — internal developer tooling, not a production concern.
#
# `_COV_FLOOR` is the enforced minimum today, not the goal. The
# stated goal (ADR-0006 §coverage) is 100% on production code. The
# floor ratchets upward in follow-up commits that close specific
# gaps; see task tracker.
#
# Ratchet history:
# - 94 (M0): initial gate landing with the parse pipeline wired up.
# - 95 (G3): F-series fills in recogniser branches + G1 adds full
#   gaiji resolve coverage; measured regions total 95.36%.
# - 96 (Cov-Ratchet): diagnostic.rs V1/V2/V3 constructor tests +
#   phase6_validate.rs synthetic-registry tests + html.rs
#   section/container/double-ruby unit tests; measured 96.07%.
#   Remaining gaps are mostly non-exhaustive `_` arms on
#   `#[non_exhaustive]` enums (structurally unreachable in-crate).
_COV_FLOOR := "96"
_COV_IGNORE := "(upstream/comrak|target/|/main\\.rs$|xtask/)"

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
# own crates. upstream/comrak is excluded (ADR-0001 keeps vendored tree
# untouched). Every check is defensive — each represents a pattern we have
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
    # #[allow(...)] / #![allow(...)] / #[cfg_attr(..., allow(...))]
    # accumulate dead rules that hide real bugs. Memory note
    # feedback_no_warning_suppression: refactor the code instead.
    check 'warning suppression (#[allow] / cfg_attr+allow)' \
        '^\s*(#!?\[allow\(|#!?\[cfg_attr\([^)]*allow\()' || failed=1

    # ---- Nightly / unstable feature gates ----------------------------------
    # We ship on Rust stable only. Feature gates silently tie us to nightly
    # and rot on toolchain bumps.
    check 'nightly feature gate (#[feature] / #![feature])' \
        '^\s*#!?\[feature\(' || failed=1

    # ---- Unsafe code -------------------------------------------------------
    # Every crate root has `#![forbid(unsafe_code)]` (checked below); this
    # text-level grep is belt-and-braces for typos that would defeat the
    # compiler gate. Excludes the legitimate `r#unsafe` raw-identifier form
    # used by comrak's `render.r#unsafe` field.
    check 'unsafe code (unsafe fn / unsafe { / unsafe impl / unsafe trait)' \
        '(^|[^a-zA-Z_#])unsafe\s+(fn|impl|trait|\{)' || failed=1

    # ---- Required deny directive -------------------------------------------
    # Each crate root must start with `#![forbid(unsafe_code)]` so accidental
    # unsafe additions are rejected at compile time.
    for root in crates/*/src/lib.rs crates/*/src/main.rs; do
        [[ -f "$root" ]] || continue
        if ! grep -q '^#!\[forbid(unsafe_code)\]' "$root"; then
            echo "==> forbidden: crate root missing '#![forbid(unsafe_code)]'" >&2
            echo "  $root" >&2
            failed=1
        fi
    done

    # ---- Toolchain pinning -------------------------------------------------
    # rust-toolchain.toml must pin a semver-numbered stable channel. Any
    # appearance of nightly/beta in the channel pin is rejected.
    if grep -qE '^\s*channel\s*=\s*"(nightly|beta)' rust-toolchain.toml; then
        echo "==> forbidden: rust-toolchain.toml pins a pre-stable channel" >&2
        grep -nE '^\s*channel' rust-toolchain.toml >&2
        failed=1
    fi

    # ---- TODO/FIXME/XXX without an issue reference -------------------------
    # Drive-by notes rot into dead reminders. Every TODO/FIXME/XXX must
    # reference either an issue (`#N`) or a milestone (`M1..M4`) so it can
    # be tracked or reclassified. Requires word-boundary match so placeholder
    # hex sequences like `U+XXXX` don't false-positive.
    todo_hits=$(grep -nE '(^|[^[:alnum:]_])(TODO|FIXME|XXX)([^[:alnum:]_]|$)' "${files[@]}" 2>/dev/null \
        | grep -vE '(#[0-9]+|M[0-9]|issue|ADR-[0-9]+)' || true)
    if [[ -n "$todo_hits" ]]; then
        echo '==> forbidden: bare TODO/FIXME/XXX without an issue or milestone reference' >&2
        echo "$todo_hits" >&2
        failed=1
    fi

    # ---- println! / eprintln! in library crates ----------------------------
    # Library crates should emit observability via `tracing`, not raw print.
    # CLI crates (afm-cli, xtask) are expected to print, so they are scoped
    # out. This complements clippy::print_stdout / clippy::print_stderr,
    # which cannot be selectively enabled per-crate while still inheriting
    # [workspace.lints] (rust-lang/cargo#12697).
    lib_files=(crates/afm-syntax/**/*.rs crates/afm-parser/**/*.rs crates/afm-encoding/**/*.rs)
    print_hits=$(grep -nE '(^|[^[:alnum:]_])e?print(ln)?!\s*\(' "${lib_files[@]}" 2>/dev/null \
        | grep -vE '/(tests|benches)/' || true)
    if [[ -n "$print_hits" ]]; then
        echo '==> forbidden: println! / eprintln! in library crates (use tracing instead)' >&2
        echo "$print_hits" >&2
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

# --- upstream / fork management ----------------------------------------------

# Report diff-line count against upstream comrak (hard fail > 200 lines)
upstream-diff:
    {{_dev}} cargo run --package xtask --quiet -- upstream-diff

# Sync upstream comrak to TAG and re-apply hook patches
upstream-sync TAG:
    {{_dev}} cargo run --package xtask --quiet -- upstream-sync {{TAG}}

# Refresh the Aozora corpus lockfile (re-pins 120 works by current SHA256)
corpus-refresh:
    {{_dev}} cargo run --package xtask --quiet -- corpus-refresh

# Regenerate `spec/*.json` from the vendored cmark-format sources under
# `spec/sources/*.txt`. Offline-pure: both the sources and the generated
# fixtures are committed to the repo. Add new `spec/sources/<name>.txt`
# files and extend the conversion block below to cover them.
spec-refresh:
    {{_dev}} bash -c '\
        set -euo pipefail && \
        cargo run --package xtask --quiet -- spec-refresh \
            --input spec/sources/commonmark-0.31.2.txt \
            --output spec/commonmark-0.31.2.json && \
        cargo run --package xtask --quiet -- spec-refresh \
            --input spec/sources/gfm-0.29-gfm.txt \
            --output spec/gfm-0.29-gfm.json'

# --- docs ---------------------------------------------------------------------

# Build the mdbook documentation site
book-build:
    docker compose run --rm book mdbook build

# Serve the mdbook site at http://localhost:3000
book-serve:
    docker compose up book

# Check documentation links
book-linkcheck:
    docker compose run --rm book mdbook-linkcheck

# New Architecture Decision Record (MADR template)
adr TITLE:
    {{_dev}} cargo run --package xtask --quiet -- new-adr {{TITLE}}

# --- end-to-end (M3 onward) --------------------------------------------------

# Playwright browser tests (Chromium + WebKit)
e2e *ARGS:
    docker compose run --rm browser \
        bash -c 'cd crates/afm-book && npm ci && npx playwright test {{ARGS}}'

# --- aggregate ----------------------------------------------------------------

# Local replica of the full CI pipeline — everything must pass before push
ci:
    just lint
    just build
    just test
    just spec-commonmark
    just spec-gfm
    just spec-aozora
    just spec-golden-56656
    just deny
    just audit
    just udeps
    just upstream-diff
    just coverage
    just book-build

# --- developer workflow helpers ----------------------------------------------

# Run after a build to verify the cache is actually warm; a first-hand
# way to notice when `RUSTC_WRAPPER` gets defeated by stray env or profile tweaks.
# Show sccache hit/miss ratio, cache size, fetch counts.
sccache-stats:
    {{_dev}} sccache --show-stats

# Useful before a measurement window:
#   just sccache-zero && just clean && just build && just sccache-stats
# Reset sccache counters to zero.
sccache-zero:
    {{_dev}} sccache --zero-stats

# Defaults to the `check` job; pass a job name to pick another, e.g.
# `just watch clippy`. Keybindings: `t` test / `c` clippy / `d` doc /
# `f` failing-only / `esc` previous job / `q` quit / Ctrl-J list jobs.
# Start the bacon file-watcher inside the dev container.
watch JOB="":
    {{_dev}} bacon {{JOB}}

# Keeps the watch loop but prints plain lines. Useful for piping output
# (`| tee`) and for sessions without a TTY.
# Headless bacon run (no TUI).
watch-headless JOB="check":
    {{_ci}} bacon --headless --job {{JOB}}

# Idempotent — re-run safely after lefthook.yml edits or to repair stubs.
# Install git hooks (pre-commit / commit-msg / pre-push).
hooks:
    {{_dev}} lefthook install

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
