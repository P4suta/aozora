# syntax=docker/dockerfile:1.7
# aozora development / CI container
# Every developer and CI job runs inside this image. Host toolchain is never invoked.
#
# Layered so dependency bumps rebuild minimal surface.

ARG RUST_VERSION=1.95.0

########################################################################
# Stage: toolchain — Rust stable + system deps for builds and CJK work
########################################################################
FROM rust:${RUST_VERSION}-bookworm AS toolchain

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && \
    apt-get install -y --no-install-recommends \
        build-essential \
        pkg-config \
        libssl-dev \
        clang \
        mold \
        curl \
        git \
        ca-certificates \
        unzip \
        xz-utils \
        locales \
    && sed -i -e 's/# \(ja_JP.UTF-8 UTF-8\)/\1/' /etc/locale.gen \
    && sed -i -e 's/# \(en_US.UTF-8 UTF-8\)/\1/' /etc/locale.gen \
    && locale-gen

ENV LANG=en_US.UTF-8 \
    LC_ALL=en_US.UTF-8 \
    RUSTUP_PERMIT_COPY_RENAME=1

# Use mold as the default linker for faster builds.
# Note: docker-compose.yml sets RUSTFLAGS / CARGO_TARGET_*_LINKER directly
# in the container env, which is what actually drives mold for compose
# runs (the env override beats this config.toml because cargo's config
# discovery is rooted at $CARGO_HOME=/workspace/.cargo, never reading
# $HOME/.cargo). This file is kept as a safety net for any direct
# `docker run` invocation that does NOT go through compose.
RUN mkdir -p /root/.cargo && printf '%s\n' \
    '[target.x86_64-unknown-linux-gnu]' \
    'linker = "clang"' \
    'rustflags = ["-C", "link-arg=-fuse-ld=mold"]' \
    > /root/.cargo/config.toml

# Pre-install every component rust-toolchain.toml + cargo-llvm-cov
# require, so the rustup channel-sync that fires on every container
# start finds nothing to download.
#
# Without this, each CI job spends ~22-30 s on `info: downloading
# 3 components` (rustfmt + clippy + rust-src per workspace
# rust-toolchain.toml) plus an extra ~30 s on `info: downloading
# component llvm-tools` in the coverage job, all of which is pure
# overhead before any cargo work can begin. Baking the components
# into the image flattens that to a sub-second rustup metadata check.
RUN rustup component add rustfmt clippy rust-src llvm-tools-preview

########################################################################
# Stage: cargo-tools — install Rust dev utilities (cached layer)
########################################################################
# Every tool below ships a prebuilt binary on its GitHub Releases page.
# Install them via `cargo-binstall`, which downloads those binaries
# directly instead of falling through `cargo install` (= source build).
#
# Numbers from a cold-cache `dev-image.yml` run on `ubuntu-latest`:
#   - source-build path (`cargo install --locked` × 17 tools): ~30-40 min
#   - binstall path (this stage):                               ~30-60 sec
#
# Source build is what burned 44 minutes on the first `book` CI job
# (commit 1e70b60), and would burn it again on any cache eviction.
# Binstall removes that failure mode at its root.
FROM toolchain AS cargo-tools

# cargo-binstall itself ships as a single static binary. Pull the
# prebuilt tarball straight from the release page rather than
# `cargo install cargo-binstall` (which would itself be a multi-minute
# source build of the very tool we're using to *avoid* source builds).
ARG BINSTALL_VERSION=1.10.22
RUN curl -L --proto '=https' --tlsv1.2 -fsSL \
    "https://github.com/cargo-bins/cargo-binstall/releases/download/v${BINSTALL_VERSION}/cargo-binstall-x86_64-unknown-linux-musl.tgz" \
    | tar -xz -C /usr/local/cargo/bin/ cargo-binstall \
    && chmod +x /usr/local/cargo/bin/cargo-binstall

# Install every dev tool via prebuilt binaries.
#
# - `--no-confirm`: skip the y/N prompt (we're in a Dockerfile).
# - `--no-symlinks`: copy binaries instead of symlinking; safer
#   across docker overlayfs and image export.
# - `--locked`: respects the `Cargo.lock` of each crate when binstall
#   does fall back to a source build.
# - The default `--strategies crate-meta-data,quick-install,compile`
#   chain is left intact: most tools resolve through the binary
#   fetchers in seconds; the few crates without a prebuilt artifact
#   on a given target (bacon's QuickInstall mirror is currently
#   flaky, for instance) silently fall back to `cargo install`.
#   The fallback is acceptable because it's now the exception, not
#   the rule — the cargo-install hot path is gone.
#
# All tools land in /usr/local/cargo/bin (cargo's default install root).
# The single-RUN form is intentional: with binstall the whole batch
# completes in under a minute, so the previous "split bacon /
# git-cliff / lychee into separate layers" trick (which existed
# purely to keep tool-version bumps from invalidating the
# multi-hour source-build layer) is no longer needed. One layer is
# simpler and the build-time cost is now tiny either way.
RUN --mount=type=cache,target=/root/.cache/binstall,sharing=locked \
    cargo binstall --no-confirm --no-symlinks --locked \
        --root /usr/local \
        cargo-nextest \
        cargo-llvm-cov \
        cargo-deny \
        cargo-audit \
        cargo-udeps \
        cargo-semver-checks \
        cargo-insta \
        cargo-release \
        cargo-edit \
        cargo-outdated \
        cargo-fuzz \
        typos-cli \
        mdbook \
        mdbook-mermaid \
        bacon \
        git-cliff \
        lychee \
        sccache

# just (task runner) installed separately; upstream provides an install script
RUN curl -fsSL https://just.systems/install.sh \
    | bash -s -- --to /usr/local/bin --tag 1.36.0

# lefthook (pre-commit manager). As of 2.x the release asset is a gzipped raw binary.
ARG LEFTHOOK_VERSION=2.1.6
RUN curl -fsSL \
    "https://github.com/evilmartians/lefthook/releases/download/v${LEFTHOOK_VERSION}/lefthook_${LEFTHOOK_VERSION}_Linux_x86_64.gz" \
    | gunzip > /usr/local/bin/lefthook \
    && chmod +x /usr/local/bin/lefthook

########################################################################
# Stage: dev — everything a contributor needs
########################################################################
FROM toolchain AS dev

COPY --from=cargo-tools /usr/local/cargo/bin/ /usr/local/cargo/bin/
COPY --from=cargo-tools /usr/local/bin/ /usr/local/bin/

# nightly toolchain is needed for cargo-udeps and cargo-fuzz harnesses
RUN rustup toolchain install nightly --component rust-src --profile minimal

ENV CARGO_HOME=/workspace/.cargo \
    CARGO_TARGET_DIR=/workspace/target \
    RUSTC_WRAPPER=sccache \
    SCCACHE_DIR=/workspace/.sccache \
    RUST_BACKTRACE=1

# Pre-create cache mount targets so the runtime volume mounts at
# /workspace/{target,.cargo,.sccache} can attach without docker
# needing to mkdirat() into a read-only `/workspace`. Without these
# the `:ro` bind mount of the source tree blocks volume attachment
# and `docker compose run --rm ci ...` fails at container start with
# "read-only file system" during mountpoint creation.
RUN mkdir -p /workspace/target /workspace/.cargo /workspace/.sccache

WORKDIR /workspace

# Default shell friendly for interactive dev sessions
CMD ["bash"]

########################################################################
# Stage: ci — same image as dev; named separately so CI pins an explicit target
########################################################################
FROM dev AS ci

########################################################################
# Stage: book — lean image for `mdbook build` / `mdbook serve` only.
# No Rust toolchain, no sccache: copies in the prebuilt mdbook +
# mdbook-mermaid + lychee binaries from `cargo-tools` and stops there.
########################################################################
FROM debian:bookworm-slim AS book

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        git \
    && rm -rf /var/lib/apt/lists/*

COPY --from=cargo-tools /usr/local/bin/mdbook         /usr/local/bin/mdbook
COPY --from=cargo-tools /usr/local/bin/mdbook-mermaid /usr/local/bin/mdbook-mermaid
COPY --from=cargo-tools /usr/local/bin/lychee         /usr/local/bin/lychee

WORKDIR /workspace/crates/aozora-book
EXPOSE 3000
CMD ["mdbook", "serve", "--hostname", "0.0.0.0", "--port", "3000"]
