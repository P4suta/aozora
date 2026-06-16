# Call from another language

**Problem.** You are not writing Rust — you want to parse Aozora
notation from Go, Java, Python, JavaScript, Ruby, PHP, or something
further down the long tail.

## One parser, many front doors

There is exactly **one parser**. Every binding funnels the same source
text through the same lexer and emits the same HTML, the same canonical
serialise, and the same [wire-envelope](../wire/overview.md) JSON —
**byte-identical across every language**. So the decision is not
"which binding is more correct"; it is "which fits the language and
runtime I already have." [Choosing a binding](../bindings/choosing.md)
is the full decision table; this recipe is the short jump list.

## Pick your language

- **JavaScript / TypeScript** (browser, Node, Deno, edge) →
  [`aozora-wasm`](../bindings/wasm.md). A `wasm-bindgen` `Document`
  class; runs client-side and at the edge, distributed on npm.

- **Python** → [`aozora-py`](../bindings/python.md). An in-process
  PyO3 native module built with maturin:

  ```python
  from aozora_py import Document
  doc = Document("｜青梅《おうめ》")
  print(doc.to_html())     # <ruby>青梅<rt>おうめ</rt></ruby>
  ```

- **Go** → [`aozora-go`](../bindings/go.md). A pure-Go
  [wazero](https://wazero.io) host over `aozora.wasm` — **no cgo, no C
  toolchain**:

  ```sh
  go get github.com/P4suta/aozora-go
  ```

- **C / C++ / Zig / any FFI-capable native language** → the
  [`aozora-ffi`](../bindings/c.md) C ABI: an opaque handle plus JSON
  over a stable C header (`aozora.h`).

- **Java, PHP, Ruby, .NET, Elixir, Haskell, … the long tail** → the
  [`aozora-extism`](../bindings/extism.md) host SDK. One portable
  `aozora.wasm` that any [Extism](https://extism.org) host SDK loads —
  see below.

- **Anything other than HTML** (EPUB, LaTeX/PDF, DOCX, …) → the
  [`aozora pandoc`](epub-pandoc.md) pipe, regardless of host language.

## The Extism template (the breadth strategy)

For the languages without a bespoke native binding, the answer is the
single `aozora.wasm` artifact loaded through that language's Extism
host SDK. The steps are identical in every SDK — only the method names
change:

1. Obtain `aozora.wasm` (a GitHub release asset).
2. Load it with your host SDK's plugin constructor (no WASI needed).
3. Assert `schema_version` matches the wire schema you compiled
   against.
4. Call an export with the source string:
   - `to_html` / `serialize` → a bare string;
   - `diagnostics_json` / `nodes_json` / `pairs_json` /
     `container_pairs_json` → a `{ schema_version, data }`
     [wire envelope](../wire/overview.md).
5. Parse the envelope `data` with types generated from the committed
   JSON Schema.

The reference host SDK ([`aozora-go`](../bindings/go.md)) is exactly
this template instantiated in Go; every other Extism SDK follows the
same shape. The full export list and the language-agnostic walkthrough
live in the [Extism chapter](../bindings/extism.md). Why a wasm plugin
for the tail rather than a native binding per language is
[ADR-0006](https://github.com/P4suta/aozora/blob/main/docs/adr/0006-polyglot-bindings-via-extism.md);
the short version is in
[Choosing a binding → In-process vs host-runtime](../bindings/choosing.md#in-process-vs-host-runtime).

## See also

- [Choosing a binding](../bindings/choosing.md) — the decision table
  and the performance ordering.
- [Extism host SDKs](../bindings/extism.md) — the wasm exports and the
  per-language template.
- [Go](../bindings/go.md) · [Python](../bindings/python.md) ·
  [WASM](../bindings/wasm.md) · [C ABI](../bindings/c.md) — the
  native / in-process bindings.
- [Wire format](../wire/overview.md) — the JSON envelope every binding
  agrees on.
