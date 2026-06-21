# Choosing a binding

aozora reaches a lot of languages, but there is only **one parser** behind
them. Every surface — the Rust library, the CLI, the wasm package, the
PyO3 module, the Go module, the C ABI, the Extism plugin — funnels the same
source text through the same lexer and renders it through the same
[`aozora::json`](../json/overview.md) authority. The HTML, the canonical
source, and the diagnostic stream are therefore **byte-identical across
every binding**. What differs between them is only the *host language* you
write in and the *overhead* you pay to cross the language boundary.

So the decision is not "which binding is more correct" — they all produce
the same bytes. It is "which one fits the language and runtime I already
have, at the cost I'm willing to pay."

## Decision table

Find the row that describes you; the rest of the page explains the
trade-offs behind it.

| You are…                                   | Use                                        | Why                                                                       | Distribution            |
| ------------------------------------------ | ------------------------------------------ | ------------------------------------------------------------------------- | ----------------------- |
| Writing Rust                               | umbrella [`aozora`](rust.md) library       | Zero-copy borrowed AST, full type safety, the fastest path. No serialise. | crates.io¹              |
| At a shell / in CI / scripting             | the [`aozora`](../ref/cli.md) binary       | `check` / `render` / `fmt` / `pandoc`, reads stdin, exits with a code.     | GitHub release          |
| In the browser, Node, or TypeScript        | [`aozora-wasm`](wasm.md)                    | wasm-bindgen `Document` class; runs client-side and at the edge.          | npm                     |
| Writing Python                             | [`aozora-py`](python.md) (PyO3)             | In-process native module via maturin; idiomatic Python API.               | build-from-source²      |
| Writing Go                                 | [`aozora-go`](go.md)                        | Pure-Go [wazero](https://wazero.io) host — no cgo, no C toolchain.        | `go get`                |
| Embedding from C / C++ / another native FFI | [`aozora-ffi`](c.md) C ABI                  | Opaque handle + JSON over a stable C header; link it like any library.    | GitHub release          |
| Writing Java, PHP, Ruby, or the long tail  | [`aozora-extism`](extism.md) host SDK       | One portable `aozora.wasm` loaded by any [Extism](https://extism.org) SDK. | GitHub release          |
| Producing anything other than HTML (EPUB, LaTeX/PDF, DOCX, …) | [`aozora pandoc`](pandoc.md) | Projects to the Pandoc AST; 50+ output formats via Pandoc writers.        | GitHub release (CLI)    |

¹ crates.io publication tracks the v1.0 API freeze; until then the git-tag
form in the [install chapter](../getting-started/install.md#rust-library)
is the canonical entry point.
² PyPI wheels are pending; pre-1.0 the Python binding builds from source
via maturin.

## In-process vs host-runtime

The bindings fall into two camps, and the split is the single most useful
lens for choosing.

**In-process / native** — the Rust library, `aozora-py`, `aozora-wasm`, and
the C ABI. The parser runs inside your process's address space. Overhead is
zero (Rust) to low (a string copy and a JSON projection at the boundary for
the others). The cost is on *our* side: each of these is a native artifact
that has to be built and published for every `(OS × arch)` pair the
ecosystem expects.

**Host-runtime** — `aozora-extism`. The parser is a single portable
`aozora.wasm`, and your language loads it through its
[Extism](https://extism.org) host SDK. You pay a JSON round-trip at the
wasm boundary (text in, a versioned JSON envelope out), and your host gains
one runtime dependency (the Extism runtime). In exchange, **we do not
maintain a native build matrix for your language** — the same wasm bytes
load identically on every platform. This is the deliberate breadth strategy
for the long tail of languages where writing and shipping a bespoke native
binding is the real cost. See
[ADR-0006](https://github.com/P4suta/aozora/blob/main/docs/adr/0006-polyglot-bindings-via-extism.md)
for the full reasoning.

Because every binding already presents an interface of "string → JSON
envelope", the serialization round-trip Extism adds is intrinsic to the
shape of the problem, not a new tax. Native bindings simply skip it by
sharing the address space.

## By language

A quick jump list:

- **Rust** → the [`aozora`](rust.md) umbrella library.
- **JavaScript / TypeScript** (browser, Node, Deno, edge) → [`aozora-wasm`](wasm.md).
- **Python** → [`aozora-py`](python.md).
- **Go** → [`aozora-go`](go.md).
- **C / C++ / Zig / any FFI-capable native language** → the [`aozora-ffi`](c.md) C ABI.
- **Java, PHP, Ruby, .NET, Elixir, Haskell, …** → the [`aozora-extism`](extism.md) host SDK.
- **None of the above / shell / CI** → the [`aozora`](../ref/cli.md) CLI binary.

## By output format

aozora's renderer emits **semantic HTML5**. The decision here is binary:

- **You want HTML.** Use the built-in renderer — `to_html()` in any library
  binding, or `aozora render` at the CLI. It is the canonical output and
  what the conformance suite gates on.
- **You want anything else** — EPUB, LaTeX/PDF, DOCX, ODT, MediaWiki, and
  ~50 more — use [`aozora pandoc`](pandoc.md). It projects the parsed tree
  into the Pandoc AST, where every Pandoc writer is one pipe away. Adding a
  new format means adding a Pandoc filter, never extending the parser.

## A note on performance

If raw throughput is the deciding factor, the ordering is:

1. **Rust, borrowed-arena.** The library hands you `Node`s that borrow
   directly from the `bumpalo` arena — no copies, no serialise, no JSON.
   Nothing is faster.
2. **In-process native bindings** (`aozora-py`, `aozora-wasm`, C ABI). One
   string copy in, one JSON projection out, but all in-process. Low,
   constant overhead.
3. **Extism.** A wasm-boundary JSON round-trip on top of the in-process
   cost. The slowest of the three *transports* — and still the right choice
   when the alternative is no binding for your language at all.

For the overwhelming majority of documents this difference is invisible
against I/O. Reach for the Rust library's borrowed AST only when you are
parsing at scale (the corpus sweep over ~17 000 works is the motivating
case); otherwise pick the binding that fits your language and let the
constant overhead disappear into the noise.

## See also

- [Rust library](rust.md) — the first-class, zero-copy binding.
- [WASM (wasm-pack)](wasm.md) — browser / Node / edge.
- [Python (PyO3 / maturin)](python.md) — the native Python module.
- [Go](go.md) — pure-Go wazero host, no cgo.
- [C ABI](c.md) — opaque handle over a stable C header.
- [Extism plugin](extism.md) — one wasm for the polyglot long tail.
- [Pandoc AST projection](pandoc.md) — every non-HTML output format.
- [JSON output](../json/overview.md) — the shared JSON envelope every
  binding agrees on.
