# aozora-wasm

WebAssembly driver for [**aozora**](https://github.com/P4suta/aozora) — a
pure-functional Rust parser for **青空文庫記法** (Aozora Bunko notation):
ruby (`｜青梅《おうめ》`), bouten (`［＃「X」に傍点］`), 縦中横, 外字
references, kaeriten, indent / align containers, page breaks, and more.

A [`wasm-bindgen`](https://rustwasm.github.io/wasm-bindgen/) surface exposing
`aozora::Document` to JavaScript / TypeScript. It powers the browser
playground and carries browser-only primitives (gaiji-at-offset, per-method
profiling, prewarm). Every JSON method delegates to `aozora::json`, so its
output is byte-identical to the FFI, Extism, and PyO3 drivers.

## Install

This crate is not published to npm; build the `pkg/` bundle with
[`wasm-pack`](https://drager.github.io/wasm-pack/):

```sh
rustup target add wasm32-unknown-unknown
wasm-pack build --target web --release crates/aozora-wasm
```

That emits `pkg/aozora_wasm.js` + `pkg/aozora_wasm_bg.wasm` (the size budget
for the artifact, post `wasm-opt -O3`, is ≤ 500 KiB). Copy `pkg/` next to your
web assets, or add it as a local dependency of your bundler.

## Quickstart

```js
import init, { Document, slugsJson, prewarm } from "./pkg/aozora_wasm.js";

await init();          // load + instantiate the .wasm module (--target web)
prewarm();             // optional: build parser tables off the first keystroke

const doc = new Document("｜青梅《おうめ》の街");

doc.toHtml();          // → semantic HTML5 with <ruby>…</ruby>
doc.toSource();        // → re-emit Aozora source text

// Structured accessors — parsed JS objects:
doc.nodes();           // classified node spans (ruby / bouten / gaiji / …)
doc.pairs();           // matched open/close pair links
doc.containerPairs();  // indent / warichu / framed / alignEnd containers
doc.diagnostics();     // lexer diagnostics

// …or the raw, byte-identical wire envelope strings shared with the
// FFI / Extism / Go / PyO3 drivers ({"schemaVersion":2,"data":[...]}):
doc.nodesJson();
doc.diagnosticsJson();

// Static slug catalogue for ［＃…］ completion menus:
slugsJson();
```

## UTF-16 source

`new Document(source)` takes a JS string (copied once into the parser's owned
`Box<str>`). Real 青空文庫 archive files are `Shift_JIS`, so decode them to a
string host-side first, e.g. `new TextDecoder("shift_jis").decode(bytes)`.
Sources whose UTF-8 length exceeds `u32::MAX` (~4 GiB) throw at construction
rather than aborting the Wasm instance — the parser core uses `u32` span
offsets.

## Performance

`prewarm()` forces one-time parser-table initialisation (SIMD backend choice +
the annotation-classifier DFA) off the first-parse critical path — call it once
right after `init()` resolves. `Document.profileJson()` returns per-method
wall-clock timings (via `performance.now()`) for the current source, and
`resolveGaijiAt(byteOffset)` bounds its scan to a window around the cursor so
inlay-hint cost is independent of document size.

## Threading

WebAssembly here is single-threaded: a `Document` lives on the JS thread that
created it (main thread or a Worker) and is freed automatically when the JS
handle is garbage-collected. For parallelism, construct a fresh `Document` per
Worker.

## Links

- **Source & issues:** <https://github.com/P4suta/aozora>
- **API reference:** <https://p4suta.github.io/aozora/api/aozora_wasm/index.html>
- **Handbook:** <https://p4suta.github.io/aozora/>
- **Changelog:** <https://github.com/P4suta/aozora/blob/main/CHANGELOG.md>

## License

`Apache-2.0 OR MIT`, at your option.
