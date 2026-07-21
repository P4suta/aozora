# aozora-extism

An [Extism](https://extism.org/) plugin driver for aozora. It builds to
one portable `aozora.wasm`, loadable from any Extism host SDK.

## Install

Every tagged release attaches the built `aozora.wasm` to the
[GitHub Release](https://github.com/P4suta/aozora/releases/latest),
alongside its `.sha256` checksum and a CycloneDX SBOM
(`aozora-extism.cdx.json`, plus its own `.sha256`). The wasm and the SBOM
each carry a build-provenance attestation, so download and verify the
released artifact rather than trust an unpinned build:

```sh
base=https://github.com/P4suta/aozora/releases/latest/download
curl -LO "$base/aozora.wasm"
curl -LO "$base/aozora.wasm.sha256"
sha256sum --check aozora.wasm.sha256
gh attestation verify aozora.wasm --repo P4suta/aozora
```

To build the same artifact from source instead:

```sh
just extism-build     # → crates/aozora-extism/dist/aozora.wasm
```

The exports are in
[`src/plugin.rs`](https://github.com/P4suta/aozora/blob/main/crates/aozora-extism/src/plugin.rs);
the JSON they return is the shape documented at
[docs.rs/aozora](https://docs.rs/aozora/latest/aozora/json/).

Exports take UTF-8. Real 青空文庫 archive files are Shift_JIS, so decode
host-side before calling in.

Part of the [aozora](https://github.com/P4suta/aozora) workspace.
Dual-licensed Apache-2.0 OR MIT.
