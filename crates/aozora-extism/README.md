# aozora-extism

An [Extism](https://extism.org/) plugin driver for aozora. It builds to
one portable `aozora.wasm`, loadable from any Extism host SDK.

Not published, so building it is the only way to get it:

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
