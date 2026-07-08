# aozora-extism

[Extism](https://extism.org/) plugin driver for
[**aozora**](https://github.com/P4suta/aozora) — a pure-functional Rust parser
for **青空文庫記法** (Aozora Bunko notation): ruby (`｜青梅《おうめ》`), bouten
(`［＃「X」に傍点］`), 縦中横, 外字 references, kaeriten, indent / align
containers, page breaks, and more.

Compiles to a **single portable `aozora.wasm`** that any language with an
Extism host SDK (Go / Java / Python / PHP / Ruby / JS / … — ~15 languages) can
load. Each export takes the Aozora source as input bytes and returns HTML or a
wire-format JSON envelope — the same "text in → bytes out" contract as the C
ABI driver, and byte-identical to it because every JSON path delegates to
`aozora::json`, the single cross-driver authority.

## Why Extism (and not just the C ABI)

The C ABI (`aozora-ffi`) reaches any language too, but every consuming language
must ship a native library built for every `(OS × arch)` pair. This crate
collapses that matrix to **one** portable `.wasm`: identical bytes on every
platform, and the per-language work shrinks to a thin host-SDK wrapper plus
types generated from the wire JSON Schema.

## Install

This crate is not published to crates.io; build the portable artifact from the
workspace:

```sh
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown -p aozora-extism
# raw artifact: target/wasm32-unknown-unknown/release/aozora_extism.wasm
```

`just extism-build` wraps that and drops a `wasm-opt`-optimised copy at
`crates/aozora-extism/dist/aozora.wasm`. Load either from any
[Extism host SDK](https://extism.org/docs/concepts/host-sdk).

## Quickstart

```js
import createPlugin from "@extism/extism";

const plugin = await createPlugin("./aozora.wasm", { useWasi: false });

const src = "｜青梅《おうめ》の街";

const html  = (await plugin.call("to_html", src)).text();   // semantic HTML5
const nodes = (await plugin.call("nodes_json", src)).text();
// nodes → {"schemaVersion":2,"data":[{ "kind":"ruby", "span":{…} }, …]}

// Assert plugin/host wire compatibility before parsing:
const version = (await plugin.call("schema_version", "")).text();  // → "2"

await plugin.close();
```

The other exports follow the same shape: `serialize` (round-trip source),
`diagnostics_json`, `pairs_json`, and `container_pairs_json`.

## The schemaVersion envelope

Every JSON export returns the cross-driver envelope

```json
{ "schemaVersion": 2, "data": [ /* …entries… */ ] }
```

`schemaVersion` bumps on any breaking change to the serialised shape. The
`schema_version` export lets a host assert wasm/SDK compatibility at load time —
call it once (with empty input) before parsing and compare against the version
your generated types expect.

## UTF-8 source

Exports take the source as input bytes (UTF-8). Real 青空文庫 archive files are
`Shift_JIS`, so decode them to UTF-8 host-side before calling. Sources longer
than `u32::MAX` (~4 GiB) surface as an Extism error rather than aborting the
Wasm instance — the parser core uses `u32` span offsets.

## Threading

An Extism `Plugin` instance is single-owner and stateless between calls (each
call parses fresh). For parallelism, the host creates one plugin instance per
worker; there is no shared parser state to synchronise.

## Links

- **Source & issues:** <https://github.com/P4suta/aozora>
- **API reference:** <https://p4suta.github.io/aozora/api/aozora_extism/index.html>
- **Handbook:** <https://p4suta.github.io/aozora/>
- **Changelog:** <https://github.com/P4suta/aozora/blob/main/CHANGELOG.md>

## License

`Apache-2.0 OR MIT`, at your option.
