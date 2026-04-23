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

# Run all lints (fmt + clippy + typos)
lint: fmt-check clippy typos

# Format check (no-write)
fmt-check:
    {{_dev}} cargo fmt --all -- --check

# Auto-format (writes)
fmt:
    {{_dev}} cargo fmt --all

# Clippy with pedantic + nursery, no suppressions tolerated
clippy:
    {{_dev}} cargo clippy --workspace --all-targets --all-features -- \
        -D warnings \
        -W clippy::pedantic \
        -W clippy::nursery

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
