# MSRV

The root `Cargo.toml` declares the supported Rust version floor in `rust-version`.
[ADR-0034](../adr/0034-separate-toolchain-channel-from-msrv.md) explains its separation from the toolchain channel and the six-month policy.

Record compatibility measurements on this page.
`just msrv-check` rejects explicit Rust versions elsewhere under `docs/contrib/`.

## The floor

Don't infer compatibility from dependency manifests alone.
`rustyline` declares no `rust-version`, so neither a manifest scan nor Cargo's version resolver establishes its actual compiler needs.
Compile the dependency graph to measure the supported version.

`just msrv-local` checks the declared version without searching for a lower version.
To measure the floor, test successively older candidates until one fails:

```sh
rustup toolchain install 1.88.0 --profile minimal
cargo +1.88.0 check --locked --ignore-rust-version \
      --workspace --all-targets --all-features --exclude aozora-extism
cargo +1.88.0 check --locked --ignore-rust-version -p aozora-extism --all-targets
```

Keep `--ignore-rust-version` when measuring candidates.
Without the flag, Cargo enforces the declared version before compilation and prevents measurement of a lower floor.

Check `aozora-extism` with default features.
Its development-only `host-smoke` feature adds wasmtime, whose declared `rust-version` exceeds the version needed in compiler measurements.
A development-only test feature doesn't determine the public version contract.

## The std-API floor

Run Clippy with a separate configuration to check standard library API requirements:

```sh
mkdir -p /tmp/msrv-probe
sed 's/^msrv = .*/msrv = "1.88.0"/' clippy.toml > /tmp/msrv-probe/clippy.toml
CLIPPY_CONF_DIR=/tmp/msrv-probe cargo clippy \
  --workspace --all-targets --all-features --exclude aozora-extism
```

`clippy::incompatible_msrv` reports standard library APIs newer than the configured floor.

Confirm the lint executes by setting the probe's `msrv` to `1.60.0` and checking for warnings.
A cached invocation that skips linting supplies no new compatibility evidence.

## Raising it

Raise the floor only when a required stable feature has no suitable implementation on an older version.
Update `rust-version` in `Cargo.toml` and `msrv` in `clippy.toml`, then run `just msrv-check`.
