# aozora

<p align="center">
  <a href="https://crates.io/crates/aozora"><img alt="crates.io" src="https://img.shields.io/crates/v/aozora.svg"></a>
  <a href="https://docs.rs/aozora"><img alt="docs.rs" src="https://img.shields.io/docsrs/aozora"></a>
  <a href="https://p4suta.github.io/aozora/"><img alt="handbook" src="https://img.shields.io/badge/handbook-mdbook-blue"></a>
  <a href="https://github.com/P4suta/aozora/blob/main/LICENSE-APACHE"><img alt="license" src="https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue"></a>
  <a href="https://p4suta.github.io/aozora/contrib/msrv.html"><img alt="msrv" src="https://img.shields.io/crates/msrv/aozora"></a>
</p>

Pure-functional Rust parser for **青空文庫記法** (Aozora Bunko
notation): ruby (`｜青梅《おうめ》`), bouten (`［＃「X」に傍点］`),
縦中横, 外字 references (`※［＃…、第3水準1-85-54］`), kunten /
kaeriten, indent / align-end containers
(`［＃ここから2字下げ］… ［＃ここで字下げ終わり］`), and
page / section breaks.

The parser is **CommonMark-free, Markdown-free** — it deals only with
the 青空文庫 notation itself. The renderer emits semantic HTML5; the
lexer reports structured diagnostics; the AST is an owned, lifetime-free
tree that can be walked in O(n).

## Quickstart

```rust,no_run
use aozora::Document;

fn main() {
    let source = std::fs::read_to_string("src.txt").unwrap();
    let doc = Document::new(source);
    let tree = doc.parse();
    println!("{}", tree.to_html());
}
```

`Document` owns a [`bumpalo`](https://docs.rs/bumpalo) arena; the
`Tree` borrows the source for the lifetime of the `Document`. Dropping
the `Document` frees the whole tree in a single step. See the
[Library quickstart](https://p4suta.github.io/aozora/getting-started/library.html)
for the lifetime model, the diagnostic stream, and the AST walk.

## Add to your project

```sh
cargo add aozora
```

Shift_JIS decoding, the CST, query DSL, JSON Schema export, and the
proptest strategies live behind cargo features — see the
[crate feature list](https://docs.rs/aozora) on docs.rs.

## One parser, every surface

There is **one parser** behind every binding: the HTML, the canonical
source, and the diagnostic stream are byte-identical across all of
them. Pick the one that fits your language and runtime.

| You are… | Use | Why |
|---|---|---|
| Writing Rust | this crate | Owned, lifetime-free AST, full type safety. |
| At a shell / in CI | the [`aozora` CLI](https://crates.io/crates/aozora-cli) | `check` / `render` / `fmt` / `pandoc`, reads stdin, exits with a code. |
| In the browser, Node, or TypeScript | [`aozora-wasm`](https://www.npmjs.com/package/aozora-wasm) (npm) | wasm-bindgen `Document` class; runs client-side and at the edge. |
| Writing Python | [`aozora-py`](https://p4suta.github.io/aozora/bindings/python.html) | In-process native module via maturin. |
| Writing Go | [`aozora-go`](https://p4suta.github.io/aozora/bindings/go.html) | Pure-Go [wazero](https://wazero.io) host — no cgo. |
| Embedding from C / C++ / native FFI | [`aozora-ffi`](https://p4suta.github.io/aozora/bindings/c.html) | Opaque handle + JSON over a stable C header. |
| Java, PHP, Ruby, or the long tail | [`aozora-extism`](https://p4suta.github.io/aozora/bindings/extism.html) | One portable `aozora.wasm` loaded by any [Extism](https://extism.org) SDK. |
| Producing EPUB / LaTeX / DOCX / … | [`aozora pandoc`](https://crates.io/crates/aozora-pandoc) | Projects to the Pandoc AST; 50+ formats via Pandoc writers. |

See [Choosing a binding](https://p4suta.github.io/aozora/bindings/choosing.html)
for the in-process-vs-host trade-offs.

## CLI

```sh
aozora check FILE.txt           # lex + report diagnostics
aozora fmt --check FILE.txt     # round-trip parse ∘ to_source check
aozora render FILE.txt          # render to HTML on stdout
aozora inspect nodes FILE.txt   # parsed nodes as JSON (pairs / gaiji / slugs …)
aozora check -E sjis FILE.txt   # Shift_JIS source from Aozora Bunko
```

Pre-built binaries for Linux / macOS / Windows are on
[the releases page](https://github.com/P4suta/aozora/releases). See the
[CLI reference](https://p4suta.github.io/aozora/ref/cli.html) for the
full subcommand surface.

## Documentation

- 🎮 [Playground](https://p4suta.github.io/aozora/playground/) — try the parser in your browser
- 📚 [Handbook](https://p4suta.github.io/aozora/) — notation reference, architecture, bindings, CLI
- 📖 [API reference](https://docs.rs/aozora) — this crate's rustdoc

The build-block crates (`aozora-spec`, `aozora-syntax`,
`aozora-pipeline`, `aozora-render`, `aozora-encoding`, …) carry no
API-stability contract of their own. This umbrella is the stable
seam: it re-exports a *curated* surface — the parsed-AST types at the
crate root (`Document`, `Tree`, `Node`, …) plus the `syntax::ast` /
`render` / `encoding` / `json` modules — never a `pub use …::*` glob,
so a refactor inside a build block cannot silently reshape what
`aozora` consumers see. Depend on `aozora` alone.

## License

Dual-licensed under
[Apache-2.0](https://github.com/P4suta/aozora/blob/main/LICENSE-APACHE)
OR [MIT](https://github.com/P4suta/aozora/blob/main/LICENSE-MIT) at your
option, matching Rust community convention.
