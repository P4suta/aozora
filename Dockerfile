# syntax=docker/dockerfile:1.7
# aozora development / CI container
# Every developer and CI job runs inside this image. Host toolchain is never invoked.
#
# Layered so dependency bumps rebuild minimal surface.
#
# Base images are pinned by immutable digest (supply-chain hardening,
# C2/F9): a tag like `rust:1.96.0-bookworm` is mutable and can be
# re-pushed, so we pin the manifest-list digest and keep the
# human-readable tag inline for legibility. Resolve a fresh digest with
# `docker buildx imagetools inspect <tag>`.
#
# The rust base is held at the SAME version as rust-toolchain.toml's
# pinned channel (the workspace MSRV) on purpose: rust-toolchain.toml
# makes every `cargo` invocation select that channel, so a base on any
# other version would ship a second, never-used toolchain (pure DL +
# image bloat). Dependabot's `docker` ecosystem is told to IGNORE the
# `rust` image (.github/dependabot.yml) so it cannot drift ahead of the
# MSRV — bump BOTH this FROM pin and rust-toolchain.toml together when
# raising the MSRV. Dependabot still bumps the `ubuntu` base of the
# `book` stage automatically.

########################################################################
# Stage: toolchain — Rust stable + system deps for builds and CJK work
########################################################################
# rust:1.96.0-bookworm (digest pinned; tag kept for humans; == rust-toolchain.toml)
FROM rust:1.96.0-bookworm@sha256:19817ead3289c8c631c73df281e18b59b172f6a31f4f563290f69cddd06c30e9 AS toolchain

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
# discovery is rooted at $CARGO_HOME=/cargo/home, never reading
# $HOME/.cargo). This file is kept as a safety net for any direct
# `docker run` invocation that does NOT go through compose.
RUN mkdir -p /root/.cargo && printf '%s\n' \
    '[target.x86_64-unknown-linux-gnu]' \
    'linker = "clang"' \
    'rustflags = ["-C", "link-arg=-fuse-ld=mold"]' \
    > /root/.cargo/config.toml

# The base toolchain == rust-toolchain.toml's pinned channel (see the
# FROM note above), so it IS the toolchain every `cargo` invocation
# selects at runtime. Pre-install every component that channel +
# cargo-llvm-cov require here, so the rustup channel-sync that fires on
# every container start finds nothing to download.
#
# Without this, each CI job spends ~22-30 s on `info: downloading
# 3 components` (rustfmt + clippy + rust-src per workspace
# rust-toolchain.toml) plus an extra ~30 s on `info: downloading
# component llvm-tools` in the coverage job, all of which is pure
# overhead before any cargo work can begin. Baking the components
# into the image flattens that to a sub-second rustup metadata check.
RUN rustup component add rustfmt clippy rust-src llvm-tools-preview

# Add wasm32-unknown-unknown so `wasm-pack build --target web
# crates/aozora-wasm` (the playground build step) runs without a host
# rustup install. The target lives in the same cached layer as the
# components above.
RUN rustup target add wasm32-unknown-unknown

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
ARG BINSTALL_VERSION=1.19.1
RUN curl -L --proto '=https' --tlsv1.2 -fsSL \
    "https://github.com/cargo-bins/cargo-binstall/releases/download/v${BINSTALL_VERSION}/cargo-binstall-x86_64-unknown-linux-musl.tgz" \
    | tar -xz -C /usr/local/cargo/bin/ cargo-binstall \
    && chmod +x /usr/local/cargo/bin/cargo-binstall

# Install every dev tool via prebuilt binaries.
#
# - `--no-confirm`: skip the y/N prompt (we're in a Dockerfile).
# - `--no-symlinks`: copy binaries instead of symlinking; safer
#   across docker overlayfs and image export.
# - `--locked`: pin to each crate's `Cargo.lock` for reproducible
#   binary selection (this batch never compiles — see strategies below).
# - `--strategies crate-meta-data,quick-install` (NO `compile`):
#   binstall's default chain ends with `compile`, which on a missing
#   prebuilt silently falls through to `cargo install --from-source`.
#   That fallback turned this RUN into a 40-minute disaster on a
#   recent dev-image rebuild (PR #17 first run) when several crates
#   came back compile-only at the same moment. By dropping `compile`
#   here, binstall instead exits non-zero — making the failure
#   surface immediately so we can react with a one-line PR (re-add
#   `compile`, pin a version, swap a tool) instead of paying tens
#   of minutes per affected build. Both `crate-meta-data` (GitHub
#   Releases) and `quick-install` (community mirror) are kept, so
#   the path is still resilient to one of the two going dark.
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
        --strategies crate-meta-data,quick-install \
        --root /usr/local \
        cargo-nextest \
        cargo-llvm-cov \
        cargo-deny \
        cargo-audit \
        cargo-shear \
        cargo-semver-checks \
        cargo-insta \
        cargo-release \
        cargo-edit \
        cargo-outdated \
        cargo-fuzz \
        typos-cli \
        mdbook \
        mdbook-mermaid \
        git-cliff \
        lychee \
        sccache \
        wasm-pack

# bacon and taplo-cli have no binstall-resolvable prebuilt for the pinned
# version, so the fail-fast main batch above rejects them — they need the
# `compile` backstop:
#   - bacon (the `just watch` compiler) ships NO prebuilt on its OWN GitHub
#     releases, but the cargo-quickinstall community mirror DOES publish one, so
#     `quick-install` fetches a prebuilt and no source build happens.
#   - taplo-cli (the `just fmt`/`fmt-check` TOML formatter) 0.10.0 is absent from
#     the quick-install mirror and carries no `[package.metadata.binstall]`, so
#     it falls through to a source `compile`.
# Kept out of the main batch so a future prebuilt-less crate can't silently turn
# the whole no-compile fail-fast batch into a slow source build. Shares the same
# binstall download cache mount as the batch above.
RUN --mount=type=cache,target=/root/.cache/binstall,sharing=locked \
    cargo binstall --no-confirm --no-symlinks --locked \
        --strategies quick-install,compile \
        --root /usr/local \
        bacon \
        taplo-cli

# tree-sitter CLI — regenerates crates/tree-sitter-aozora/src/{parser.c,
# grammar.json,node-types.json} from grammar.js. Pinned to the SAME version
# as the `tree-sitter` runtime crate (Cargo.lock: 0.26.10) so the generated
# parse tables target the ABI that aozora-lsp links against, and so
# `xtask conformance grammar --check` (the grammar regen drift gate) is
# reproducible across machines.
#
# Built from source rather than binstalled: tree-sitter's prebuilt Linux
# release binary is linked against glibc 2.39 and dies at runtime on this
# bookworm base (glibc 2.36) with `GLIBC_2.39 not found` — the same
# newer-glibc problem the `book` stage documents for lychee, and there is no
# musl-static or quick-install-mirror build for 0.26.10. A source build links
# against the image's own glibc and takes ~75 s (it embeds a QuickJS engine
# via rquickjs to evaluate grammar.js, so no node is needed at generate time).
# The `--locked` flag pins the crate's own dependency set for reproducibility.
# Installs the `tree-sitter` binary into /usr/local/bin (copied into the dev
# stage alongside the other tools).
ARG TREE_SITTER_CLI_VERSION=0.26.10
RUN cargo install "tree-sitter-cli@${TREE_SITTER_CLI_VERSION}" --locked --root /usr/local \
    && tree-sitter --version

# just (task runner) installed separately; upstream provides an install script
RUN curl -fsSL https://just.systems/install.sh \
    | bash -s -- --to /usr/local/bin --tag 1.51.0

# lefthook (pre-commit manager). As of 2.x the release asset is a gzipped raw binary.
ARG LEFTHOOK_VERSION=2.1.9
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

# The stable toolchain is fully provisioned in the `toolchain` stage:
# because the base image == rust-toolchain.toml's pinned channel (see the
# FROM note), the base default IS the runtime toolchain, and the
# `toolchain` stage already added every component (rustfmt, clippy,
# rust-src, llvm-tools-preview) and the wasm32-unknown-unknown target to
# it. So there is no second toolchain to install here — only nightly,
# below. (Previously the base drifted ahead of the pin and a whole second
# pinned toolchain had to be re-installed in this stage; holding base ==
# pin removes that download and the dead base-default toolchain entirely.)

# nightly toolchain is needed for the cargo-fuzz harnesses
RUN rustup toolchain install nightly --component rust-src --profile minimal

# Bun for the playground frontend. The upstream installer drops the
# binary under $HOME/.bun; move it into /usr/local/bin so any user
# (root or non-root) finds it on PATH without sourcing a shell rc
# file. Pinned to a recent stable release so cache invalidation only
# happens on intentional bumps.
ARG BUN_VERSION=1.3.14
RUN curl -fsSL https://bun.sh/install \
    | bash -s -- "bun-v${BUN_VERSION}" \
    && mv /root/.bun/bin/bun /usr/local/bin/bun \
    && chmod +x /usr/local/bin/bun \
    && rm -rf /root/.bun

# Binaryen's wasm-opt — used by `just extism-build` to shrink the
# portable Extism plugin (and available for any other wasm
# post-processing). Pulled from the upstream GitHub release rather than
# apt on purpose: Debian bookworm's binaryen predates version 119 and
# cannot validate the bulk-memory opcodes (memory.copy / memory.fill)
# that Rust 1.95+ emits, so the apt build would reject our artifacts.
# Extracted to /opt and symlinked onto PATH; the binary's rpath
# ($ORIGIN/../lib) resolves libbinaryen.so after the symlink is followed.
ARG BINARYEN_VERSION=130
RUN curl -L --proto '=https' --tlsv1.2 -fsSL \
    "https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN_VERSION}/binaryen-version_${BINARYEN_VERSION}-x86_64-linux.tar.gz" \
    | tar -xz -C /opt \
    && ln -s "/opt/binaryen-version_${BINARYEN_VERSION}/bin/wasm-opt" /usr/local/bin/wasm-opt \
    && wasm-opt --version

# Node.js + quicktype — used by `just types-langs` to generate native
# wire types for every host-SDK language from the committed JSON Schema
# (one generator, all languages). We install Node 22 LTS from NodeSource:
# Debian bookworm ships nodejs 18.x, which is EOL and below the
# `engines.node >= 20.19` that vitest 4 (the playground test runner via
# `just playground-test`) requires — on Node 18 vitest 4 fails at startup
# because `node:util` has no `styleText` export. Node 22 also satisfies
# quicktype's `engines.node >= 18.12`; both are version-pinned so the
# drift-gated codegen stays reproducible.
ARG NODE_MAJOR=22
ARG QUICKTYPE_VERSION=23.2.6
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl gnupg \
    && curl -fsSL --proto '=https' --tlsv1.2 "https://deb.nodesource.com/setup_${NODE_MAJOR}.x" | bash - \
    && apt-get install -y --no-install-recommends nodejs \
    && rm -rf /var/lib/apt/lists/* \
    && npm install -g "quicktype@${QUICKTYPE_VERSION}" \
    && npm cache clean --force \
    && node --version \
    && quicktype --version

# Go toolchain — for the aozora-go host package (the Extism Go SDK is
# pure-Go via wazero: no cgo, no native libextism) and for gofmt'ing the
# quicktype-generated Go wire types so the committed artifact stays
# gofmt-clean. Official tarball, pinned. Module + build caches are
# redirected into the cargo-target volume (see ENV below) so they persist
# across `--rm` runs instead of re-downloading every invocation.
ARG GO_VERSION=1.26.4
RUN curl -L --proto '=https' --tlsv1.2 -fsSL \
    "https://go.dev/dl/go${GO_VERSION}.linux-amd64.tar.gz" \
    | tar -xz -C /usr/local \
    && ln -s /usr/local/go/bin/go /usr/local/bin/go \
    && ln -s /usr/local/go/bin/gofmt /usr/local/bin/gofmt \
    && go version

ENV CARGO_HOME=/cargo/home \
    CARGO_TARGET_DIR=/cargo/target \
    RUSTC_WRAPPER=sccache \
    SCCACHE_DIR=/cargo/sccache \
    GOCACHE=/cargo/target/go-build \
    GOMODCACHE=/cargo/target/go-mod \
    RUST_BACKTRACE=1

# Pre-create the cache mount targets at /cargo/* so the named volume
# mounts attach cleanly. These live OUTSIDE the /workspace bind mount
# on purpose (see docker-compose.yml): nesting them under /workspace
# made the daemon create root-owned ./target / ./.cargo / ./.sccache
# on the host, which broke host-side cargo (`smoke-ffi` / `pgo`).
RUN mkdir -p /cargo/target /cargo/home/registry /cargo/home/git /cargo/sccache \
    /workspace/playground/node_modules /workspace/playground/dist

# Run as a non-root user so files written into the /workspace bind mount
# (wasm pkg/, generated docs, coverage reports, playground node_modules/
# dist) are owned by the host developer, not root — the classic bind-mount
# footgun. UID/GID default to the conventional first-user 1000; override
# with `--build-arg UID=$(id -u) --build-arg GID=$(id -g)` on hosts that
# differ. Debian bookworm's base leaves 1000 free (the `book` stage's
# ubuntu base does not — it reuses the existing `ubuntu` user). The cache
# dirs at /cargo/* and the playground mountpoints are chowned so a fresh
# named volume initialises dev-owned. CI flips the runtime UID back to
# root via `user:` in docker-compose.yml (AOZORA_UID=0, set by
# setup-dev-image): the runner's checkout is owned by a different UID and
# ownership is throwaway on the ephemeral runner.
ARG UID=1000
ARG GID=1000
RUN groupadd --gid "${GID}" dev \
    && useradd --uid "${UID}" --gid "${GID}" --create-home --shell /bin/bash dev \
    && chown -R "${UID}:${GID}" /cargo /workspace
ENV HOME=/home/dev

WORKDIR /workspace
USER dev

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
#
# Base is ubuntu:24.04 (glibc 2.39) rather than debian:bookworm-slim
# (glibc 2.36) because lychee's prebuilt binaries on its GitHub
# Releases page have been built against newer glibc since 0.20.x —
# they fail at runtime on bookworm with `GLIBC_2.38 / 2.39 not found`.
# The mdbook / mdbook-mermaid binaries from cargo-tools (built on
# debian:bookworm) keep working here because glibc is forwards-
# compatible: an older-glibc-built ELF runs fine on a newer glibc.
#
# Pinned by digest (see the toolchain-stage note above); Dependabot's
# docker ecosystem bumps the tag + sha together. Refresh via
# `docker buildx imagetools inspect ubuntu:26.04`.
########################################################################
# ubuntu:26.04 (digest pinned; tag kept for humans / Dependabot)
FROM ubuntu:26.04@sha256:b7f48194d4d8b763a478a621cdc81c27be222ba2206ca3ca6bc42b49685f3d9e AS book

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

# Run mdbook as non-root so book output written into the /workspace bind
# mount is host-owned, not root. The ubuntu:24.04 base already ships a
# UID/GID-1000 `ubuntu` user, so reuse it (matches the dev stage's UID
# 1000) rather than creating a colliding `dev` user. The CI `book` job
# invokes this image via raw `docker run --user 0:0` (root) because its
# checkout is runner-owned; local `docker compose run book` keeps 1000.
ENV HOME=/home/ubuntu

WORKDIR /workspace/crates/aozora-book
USER ubuntu
EXPOSE 3000
CMD ["mdbook", "serve", "--hostname", "0.0.0.0", "--port", "3000"]
