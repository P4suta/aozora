# MSRV

The contract is `rust-version` in the root `Cargo.toml`. Why it is a
different number from the toolchain channel, and the six-month rule it
obeys, are in
[ADR-0034](../adr/0034-separate-toolchain-channel-from-msrv.md).

This page holds the measurements, which no recipe can perform for you.
It is the only page under `docs/contrib/` allowed to name a Rust
version; `just msrv-check` fails the rest.

## The floor

Do not infer it from dependency manifests. `rustyline` declares no
`rust-version` at all — invisible to a manifest sweep and to Cargo's
MSRV-aware resolver, perfectly visible to the compiler. The floor is
whatever actually compiles.

`just msrv-local` confirms the *declared* version builds. It cannot find
a lower one. To re-measure, walk candidates down until one fails:

```sh
rustup toolchain install 1.88.0 --profile minimal
cargo +1.88.0 check --locked --ignore-rust-version \
      --workspace --all-targets --all-features --exclude aozora-extism
cargo +1.88.0 check --locked --ignore-rust-version -p aozora-extism --all-targets
```

`--ignore-rust-version` is not optional. Without it Cargo enforces the
declared `rust-version` before the compiler gets a say, so you would only
be testing the declaration.

`aozora-extism` is checked on default features: its dev-only `host-smoke`
feature pulls wasmtime, which declares a higher `rust-version` than it
needs. A dev-only test feature does not set a public contract.

## The std-API floor

Ask clippy, without touching the checked-in config:

```sh
mkdir -p /tmp/msrv-probe
sed 's/^msrv = .*/msrv = "1.88.0"/' clippy.toml > /tmp/msrv-probe/clippy.toml
CLIPPY_CONF_DIR=/tmp/msrv-probe cargo clippy \
  --workspace --all-targets --all-features --exclude aozora-extism
```

`clippy::incompatible_msrv` then names every std API newer than that
floor, with line numbers.

Confirm the lint is alive before believing a clean run: set `msrv` to
`1.60.0` and check warnings appear. A cached run that never re-lints
looks exactly like a clean one.

## Raising it

Only when a stable feature you need cannot be expressed by anything
older. Set `rust-version` in `Cargo.toml` and `msrv` in `clippy.toml`,
then `just msrv-check`.
