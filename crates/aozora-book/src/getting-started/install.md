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

Each archive also bundles shell completion scripts under `completions/`
(bash / zsh / fish / powershell / elvish / nushell) and man pages under
`man/man1/`; see
[CLI reference → completions](../ref/cli.md#aozora-completions) for where
to install the completions. Regenerate either at any time with
`aozora completions <shell>` or `aozora man [SUBCOMMAND]`.

## CLI binary (build from source)

The released CLI is on crates.io — `cargo install` compiles it from the
published source:

```sh
cargo install aozora-cli --locked
```

The `--locked` flag is non-negotiable — it pins to the exact
`Cargo.lock` we shipped, which matters because the workspace uses fat
LTO (mismatched dep versions silently change inlining behaviour).

To track the development tip instead, install from git:

```sh
cargo install --git https://github.com/P4suta/aozora --locked aozora-cli
```

Or pin a specific release tag (the current value is on
[the releases page](https://github.com/P4suta/aozora/releases/latest)):

```sh
cargo install --git https://github.com/P4suta/aozora \
              --tag v0.4.1 --locked aozora-cli
```

## Rust library

aozora is on crates.io. Depend on the umbrella crate alone — it is the
single front door, and the build-block crates (`aozora-encoding`, …)
are reached through its re-exports (`aozora::encoding`, …):

```toml
[dependencies]
aozora = "0.4"
```

**Bleeding-edge alternative** — to track unreleased fixes on `main`,
pin a git tag instead. **This block is the single source of truth for
the recommended git pin** — every other doc links here, so a new
release only needs this one tag updated:

```toml
[dependencies]
aozora = { git = "https://github.com/P4suta/aozora.git", tag = "v0.4.1" }
```

The current tag is whatever
[GitHub Releases](https://github.com/P4suta/aozora/releases/latest) is
marked **Latest**. Either way the repo follows Conventional Commits and
SemVer: breaking changes advance the *major* version (post-1.0) or the
*minor* version (during 0.x), so a `"0.4"` requirement stays safe.

## WASM (browser / Node)

The browser package is on npm as
[`aozora-wasm`](https://www.npmjs.com/package/aozora-wasm):

```sh
npm install aozora-wasm
```

To build it from a checkout instead:

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

The wheel is on PyPI as
[`aozora_py`](https://pypi.org/project/aozora_py/):

```sh
pip install aozora_py
```

For local development against a checkout, build with maturin instead:

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
