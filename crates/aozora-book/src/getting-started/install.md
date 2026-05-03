# Install

aozora ships in five shapes — pick the one that matches how you want
to consume the parser.

## CLI binary (release archive)

Pre-built `aozora` binaries for the three Tier-1 platforms ride on
every GitHub Release:

- `aozora-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz`
- `aozora-vX.Y.Z-aarch64-apple-darwin.tar.gz`
- `aozora-vX.Y.Z-x86_64-pc-windows-msvc.zip`

Each archive is shipped with a `SHA256SUMS` companion. Browse them at
<https://github.com/P4suta/aozora/releases>.

```sh
curl -L -O \
  https://github.com/P4suta/aozora/releases/latest/download/aozora-x86_64-unknown-linux-gnu.tar.gz
tar -xzf aozora-*.tar.gz
sudo install -m 0755 aozora /usr/local/bin/
aozora --version
```

## CLI binary (build from source)

Cargo can build the CLI directly from the repository. The `--locked`
flag is non-negotiable — it pins to the exact `Cargo.lock` we shipped,
which matters because the workspace uses fat LTO (mismatched dep
versions silently change inlining behaviour).

Latest `main` (default — tracks the development tip):

```sh
cargo install --git https://github.com/P4suta/aozora --locked aozora-cli
```

Reproducible build pinned to a release tag (replace the tag with the
current value from
[the releases page](https://github.com/P4suta/aozora/releases/latest)):

```sh
cargo install --git https://github.com/P4suta/aozora \
              --tag v0.3.0 --locked aozora-cli
```

## Rust library

aozora is not yet on crates.io — public release tracks the v1.0 API
freeze. Until then, depend on a tagged commit. **This snippet is the
single source of truth for the recommended pin** — every other doc
link here instead of inlining the tag, so a new release only needs
this one block updated:

```toml
[dependencies]
aozora          = { git = "https://github.com/P4suta/aozora.git", tag = "v0.3.0" }
aozora-encoding = { git = "https://github.com/P4suta/aozora.git", tag = "v0.3.0" }
```

The current tag is whatever
[GitHub Releases](https://github.com/P4suta/aozora/releases/latest) is
marked **Latest**; bump the two `tag = "..."` lines accordingly.

Ship-it pattern: pin the tag in `Cargo.toml`, let Dependabot bump it
on the next release. The repo follows Conventional Commits and
SemVer; breaking changes always advance the *major* version (post-1.0)
or the *minor* version (during 0.x).

## WASM (browser / Node)

```sh
rustup target add wasm32-unknown-unknown        # one-time
wasm-pack build --target web --release crates/aozora-wasm
```

The post-`wasm-opt` artifact has a 500 KiB size budget. See
[Bindings → WASM](../bindings/wasm.md) for the JS surface and the
post-build `wasm-opt` invocation we recommend.

## C ABI

```sh
cargo build --release -p aozora-ffi
# → target/release/libaozora_ffi.{so,dylib,a}
# → target/release/aozora.h          (cbindgen-generated)
```

Link with `-laozora_ffi` and include `aozora.h`. See
[Bindings → C ABI](../bindings/c.md) for the API surface and memory
ownership rules.

## Python

```sh
pip install maturin                              # one-time
cd crates/aozora-py
maturin develop -F extension-module              # install in current venv
maturin build   -F extension-module --release    # produce a redistributable wheel
```

See [Bindings → Python](../bindings/python.md) for the API and the
`unsendable` thread-safety contract.

## Toolchain pin

aozora pins **Rust 1.95.0** as its MSRV (`rust-toolchain.toml`). CI
enforces it via a dedicated `msrv` job. If you run `rustup show`
inside the repo and see something else, your local override needs
updating.
