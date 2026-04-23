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

# Fuzz smoke (60s per harness) — runs the registered cargo-fuzz harnesses
fuzz *ARGS:
    {{_dev}} bash -c 'cd crates/afm-parser && cargo +nightly fuzz run {{ARGS}}'

# Benchmarks (criterion)
bench *ARGS:
    {{_dev}} cargo bench --workspace {{ARGS}}

# --- coverage -----------------------------------------------------------------

# Branch coverage (C1). Fails if <100% on our code (upstream comrak excluded).
coverage:
    {{_dev}} cargo llvm-cov nextest \
        --branch \
        --workspace \
        --ignore-filename-regex '(upstream/comrak|target/)' \
        --fail-under-branches 100

# HTML coverage report for local inspection
coverage-html:
    {{_dev}} cargo llvm-cov nextest \
        --branch \
        --workspace \
        --ignore-filename-regex '(upstream/comrak|target/)' \
        --html --output-dir coverage/html

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

# --- cleanup ------------------------------------------------------------------

# Remove build artifacts (keeps volumes; use `docker compose down -v` for volumes)
clean:
    {{_dev}} cargo clean --workspace

# Tear down all compose state (destroys cached registry/target/sccache volumes)
nuke:
    docker compose down -v --remove-orphans
