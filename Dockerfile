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

# Use mold as the default linker for faster builds
RUN mkdir -p /root/.cargo && printf '%s\n' \
    '[target.x86_64-unknown-linux-gnu]' \
    'linker = "clang"' \
    'rustflags = ["-C", "link-arg=-fuse-ld=mold"]' \
    > /root/.cargo/config.toml

########################################################################
# Stage: cargo-tools — install Rust dev utilities (cached layer)
########################################################################
FROM toolchain AS cargo-tools

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/tmp/cargo-build \
    CARGO_TARGET_DIR=/tmp/cargo-build \
    cargo install --locked --root /usr/local \
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
        sccache

# bacon is intentionally installed in its own layer so version bumps don't
# invalidate the expensive 14-tool install above (cargo-udeps / semver-checks
# alone take >15 min from scratch because they transitively depend on cargo).
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/tmp/cargo-build \
    CARGO_TARGET_DIR=/tmp/cargo-build \
    cargo install --locked --root /usr/local bacon

# git-cliff for CHANGELOG generation. Same-reasoning as bacon: kept
# in its own layer so version bumps don't invalidate the big install.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/tmp/cargo-build \
    CARGO_TARGET_DIR=/tmp/cargo-build \
    cargo install --locked --root /usr/local git-cliff

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
