# WASM (wasm-pack)

The `aozora-wasm` crate compiles to `wasm32-unknown-unknown` and
exposes a `Document` class via `wasm-bindgen`. The wasm artifact has
a hard 500 KiB size budget after `wasm-opt -O3` — measured on every
release.

## Build

```sh
rustup target add wasm32-unknown-unknown        # one-time
wasm-pack build --target web --release crates/aozora-wasm
```

Outputs land at `crates/aozora-wasm/pkg/`:

- `aozora_wasm_bg.wasm` — the binary module
- `aozora_wasm.js` — the wasm-bindgen JS shim
- `aozora_wasm.d.ts` — TypeScript types
- `package.json` — minimal npm-publishable metadata

### Why `wasm-opt = false` in `Cargo.toml`?

`wasm-pack` ships its own bundled `wasm-opt` (via the `binaryen` crate)
which lags upstream. Recent Rust releases emit bulk-memory opcodes
(`memory.copy`, `memory.fill`) that the bundled `wasm-opt` mishandles
on `-O3`, occasionally producing artifacts that crash on `init`. We
disable the bundled run and recommend a fresh `wasm-opt` invocation
externally:

```sh
wasm-opt -O3 \
    --enable-bulk-memory \
    --enable-mutable-globals \
    crates/aozora-wasm/pkg/aozora_wasm_bg.wasm \
    -o crates/aozora-wasm/pkg/aozora_wasm_bg.wasm
```

The post-`wasm-opt` artifact has a 500 KiB size budget. CI gates on
this number — exceeding it is a release-blocking regression.

## Usage

```js
import init, { Document } from "./pkg/aozora_wasm.js";

await init();                                  // load the .wasm

const doc = new Document("｜青梅《おうめ》");
const html = doc.to_html();
const canonical = doc.serialize();
const diagnostics = JSON.parse(doc.diagnostics_json());
console.log(html);
doc.free();                                    // release the bumpalo arena
```

In TypeScript, the `.d.ts` file gives you full type checking on
every method.

## API surface

| Method | Returns | Notes |
|---|---|---|
| `new Document(source: string)` | `Document` | Copies the JS string into a Rust `Box<str>`. |
| `to_html()` | `string` | Renders to semantic HTML5 with `aozora-*` class hooks. |
| `serialize()` | `string` | Re-emits canonical 青空文庫 source. |
| `diagnostics_json()` | `string` | JSON-encoded array of diagnostic objects. |
| `source_byte_len()` | `number` | Source byte length, useful for progress UI. |
| `free()` | — | Explicit drop; otherwise the JS GC eventually releases. |

The diagnostics JSON shape mirrors `aozora-ffi`'s C ABI:

```ts
interface Diagnostic {
    code:    string;            // "E0001", "W0006", …
    level:   "error" | "warning" | "info";
    message: string;
    span:    { start: number; end: number };
    help?:   string;
}
```

## Why a hand-written JSON projection over `serde-wasm-bindgen`?

`serde-wasm-bindgen` would let us pass the `Diagnostic` directly to
JS as a structured object — no JSON round-trip needed. We don't use
it because:

- It pulls in a meaningful chunk of `serde_json` machinery that
  bloats the wasm bundle by ~80 KiB.
- The wire format (`{ code: "E0001", level: "warning", … }`) is
  exactly what every JS consumer is going to deserialise into
  anyway.
- It would force a `serde::Serialize` derivation on every
  diagnostic-related type in `aozora-spec`, which the Rust library
  consumers don't otherwise need (they take `&[Diagnostic]`
  directly).

A small, hand-written JSON emitter (one `core::fmt::Write` impl, ~60
LOC) costs nothing and keeps the bundle small.

## Why `Document.free()` and not just GC?

wasm-bindgen does wire `Drop` to a JS finalizer, but JS finalizers
fire on the GC's schedule — which can be *minutes* after the last
reference goes out of scope, especially on Node.js where the GC
batches aggressively. For large documents this means the bumpalo
arena (potentially several MB) sits unreleased.

Explicit `.free()` is the same idiom every wasm-bindgen library
exposes for resource-heavy types. Consumers that want JS-native
ergonomics wrap the class in their own `using` (TC39 stage-3 explicit
resource management) helper.

## Browser support

Tier-1 (CI-tested):

- Chrome 110+
- Firefox 110+
- Safari 16+

Tier-2 (works, not in CI):

- Node.js 18+ (use `--target nodejs` in `wasm-pack build`)
- Deno 1.30+

The bundle uses bulk-memory and mutable-globals; both have been
universally supported since 2021.

## Why wasm at all?

The CLI and the Rust library cover Linux / macOS / Windows native;
the wasm build covers everywhere else — particularly:

- Browser-side preview / formatter for a 青空文庫 LSP front-end.
- Cloudflare Workers / Vercel Edge / Deno Deploy serverless
  rendering.
- Notebook environments (Jupyter via `pyodide`, Observable, Quarto).

The same parser, same diagnostics, same canonical-serialise — across
every wasm-runtime host.

## See also

- [Install](../getting-started/install.md#wasm-browser--node)
- [Architecture → SIMD scanner backends](../arch/scanner.md) — the
  wasm32 scanner backend.
