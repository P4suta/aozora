# aozora

<p align="center">
  <a href="https://github.com/P4suta/aozora/actions/workflows/ci.yml"><img alt="ci" src="https://github.com/P4suta/aozora/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/P4suta/aozora/actions/workflows/docs.yml"><img alt="docs deploy" src="https://github.com/P4suta/aozora/actions/workflows/docs.yml/badge.svg"></a>
  <a href="https://github.com/P4suta/aozora/releases/latest"><img alt="latest release" src="https://img.shields.io/github/v/release/P4suta/aozora?display_name=tag&sort=semver"></a>
  <a href="./LICENSE-APACHE"><img alt="license" src="https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue"></a>
  <a href="./rust-toolchain.toml"><img alt="msrv" src="https://img.shields.io/badge/rust-1.96-orange"></a>
  <a href="https://scorecard.dev/viewer/?uri=github.com/P4suta/aozora"><img alt="OpenSSF Scorecard" src="https://api.securityscorecards.dev/projects/github.com/P4suta/aozora/badge"></a>
</p>

<p align="center">
  🎮 <a href="https://p4suta.github.io/aozora/playground/"><strong>Playground</strong></a>
  · 📚 <a href="https://p4suta.github.io/aozora/"><strong>Handbook (mdbook)</strong></a>
  · 📖 <a href="https://p4suta.github.io/aozora/api/aozora/"><strong>API reference (rustdoc)</strong></a>
  · 📦 <a href="https://github.com/P4suta/aozora/releases"><strong>Releases &amp; binaries</strong></a>
  · 🇯🇵 <a href="./README.ja.md"><strong>日本語</strong></a>
</p>

Pure-functional Rust parser for **青空文庫記法** (Aozora Bunko notation):
ruby (`｜青梅《おうめ》`), bouten (`［＃「X」に傍点］`), 縦中横, 外字
references (`※［＃…、第3水準1-85-54］`), kunten / kaeriten,
indent / align-end containers (`［＃ここから2字下げ］… ［＃ここで字下げ終わり］`),
and page / section breaks.

The parser is **CommonMark-free, Markdown-free** — this repository deals
only with the 青空文庫 notation itself. The renderer emits semantic HTML5;
the lexer reports structured diagnostics; the AST is an owned,
lifetime-free tree that can be walked in O(n).

## Installation

### Pre-built CLI

Pre-built `aozora` CLI binaries for **Linux x86_64**, **macOS arm64**,
and **Windows x86_64** are attached to every GitHub Release —
[the releases page](https://github.com/P4suta/aozora/releases) carries
`aozora-vX.Y.Z-<target>.{tar.gz,zip}` archives with `SHA256SUMS`.

### Build from source

```sh
cargo install --git https://github.com/P4suta/aozora --locked aozora-cli
```

(builds the latest `main`; pin to a release tag for reproducible builds —
see [the install chapter](https://p4suta.github.io/aozora/getting-started/install.html)
for the tag-pinned form.)

### As a Rust library

The `Cargo.toml` snippet (with the current release tag) lives in the
[install chapter](https://p4suta.github.io/aozora/getting-started/install.html#as-a-rust-library) —
keeping it in one place avoids version-pin drift across multiple READMEs.
crates.io publication tracks the 1.0 API freeze.

For WASM / C ABI / Python bindings see the
[Bindings chapters](https://p4suta.github.io/aozora/bindings/rust.html) of
the handbook.

### WASM / npm

```sh
npm install aozora-wasm
```

```js
import init, { Document } from "aozora-wasm";

await init();
const html = new Document("｜青梅《おうめ》").toHtml();
```

See the [WASM bindings chapter](https://p4suta.github.io/aozora/bindings/wasm.html)
for the full `Document` method surface and the diagnostics JSON shape.

## Quickstart

```rust
use aozora::Document;

let source = "｜青梅《おうめ》".to_owned();
let doc = Document::new(source);
let tree = doc.parse();

let html: String = tree.to_html();
let canonical: String = tree.to_source();
let diagnostics = tree.diagnostics();

// `to_source` re-serialises to the *canonical* form: the leading `｜`
// is redundant for an all-kanji base, so it is dropped (ADR 0003).
assert_eq!(canonical, "青梅《おうめ》");
// `to_source_verbatim` instead replays the author's exact bytes.
assert_eq!(tree.to_source_verbatim(), "｜青梅《おうめ》");
```

`Document` owns a [`bumpalo`](https://docs.rs/bumpalo) arena; `tree`
borrows from it for the lifetime of the `Document`. Dropping the
`Document` releases every node in a single `Bump::reset` step.

## Choosing a binding

There is **one parser** behind every surface: the HTML, the canonical
source, and the diagnostic stream are byte-identical across all of
them. Pick the one that fits the language and runtime you already have.

| You are… | Use | Why |
|---|---|---|
| Writing Rust | umbrella [`aozora`](./crates/aozora) library | Owned, lifetime-free AST, full type safety — the fastest path. |
| At a shell / in CI | the `aozora` CLI | `check` / `render` / `fmt` / `pandoc`, reads stdin, exits with a code. |
| In the browser, Node, or TypeScript | [`aozora-wasm`](./crates/aozora-wasm) (npm) | wasm-bindgen `Document` class; runs client-side and at the edge. |
| Writing Python | [`aozora-py`](./crates/aozora-py) (PyO3) | In-process native module via maturin; idiomatic Python API. |
| Writing Go | [`aozora-go`](./crates/aozora-go) | Pure-Go [wazero](https://wazero.io) host — no cgo, no C toolchain. |
| Embedding from C / C++ / another native FFI | [`aozora-ffi`](./crates/aozora-ffi) C ABI | Opaque handle + JSON over a stable C header; link it like any library. |
| Writing Java, PHP, Ruby, or the long tail | [`aozora-extism`](./crates/aozora-extism) host SDK | One portable `aozora.wasm` loaded by any [Extism](https://extism.org) SDK. |
| Producing anything other than HTML (EPUB, LaTeX/PDF, DOCX, …) | [`aozora pandoc`](./crates/aozora-pandoc) | Projects to the Pandoc AST; 50+ output formats via Pandoc writers. |

See the [Choosing a binding](https://p4suta.github.io/aozora/bindings/choosing.html)
chapter of the handbook for the in-process-vs-host-runtime trade-offs and
the per-language jump list.

## CLI

```sh
aozora check FILE.txt           # lex + report diagnostics
aozora fmt --check FILE.txt     # round-trip parse ∘ to_source check
aozora render FILE.txt          # render to HTML on stdout
aozora inspect nodes FILE.txt   # parsed nodes as JSON (pairs / gaiji …)
aozora check -E sjis FILE.txt   # Shift_JIS source from Aozora Bunko
```

All subcommands accept `-` (or no path argument) to read from stdin.
See the [CLI reference chapter](https://p4suta.github.io/aozora/ref/cli.html)
for the full subcommand reference.

## Crate layout

aozora is a multi-crate workspace (plus the `aozora-go` Go module).
[`crates/aozora`](./crates/aozora) is the public facade — library
consumers usually import only this one.

| Crate | Purpose |
|---|---|
| [`crates/aozora`](./crates/aozora) | Top-level facade. `Document::parse() → Tree<'_>`, structured `Diagnostic`s, `SLUGS` catalogue, `canonicalise_slug`. The single front door. |
| [`crates/aozora-spec`](./crates/aozora-spec) | Single source of truth for shared types: `Span`, `TriggerKind`, `PairKind`, `Diagnostic`, PUA sentinel codepoints, `SLUGS` dispatch table. No internal dependency. |
| [`crates/aozora-syntax`](./crates/aozora-syntax) | AST types (`Node` variants, `ContainerKind`, `BoutenKind`, `Indent`). |
| [`crates/aozora-encoding`](./crates/aozora-encoding) | Shift_JIS decoding + 外字 lookup (compile-time PHF, JIS X 0213 + UCS resolution). |
| [`crates/aozora-scan`](./crates/aozora-scan) | SIMD-friendly multi-pattern scanner backends (Teddy / structural-bitmap / Hoehrmann DFA / naive fallback). |
| [`crates/aozora-veb`](./crates/aozora-veb) | Eytzinger-layout sorted-set lookup (cache-friendly binary search). |
| [`crates/aozora-pipeline`](./crates/aozora-pipeline) | Lexer (sanitize → tokenize → pair → classify) plus the `lex` orchestrator — pure `fn(&str, &Arena) -> LexOutput<'_>`. |
| [`crates/aozora-render`](./crates/aozora-render) | HTML and serialise renderers — `html::render_to_string`, `serialize::serialize`. |
| [`crates/aozora-cst`](./crates/aozora-cst) | rowan-backed lossless concrete syntax tree. Editor/formatter surface. |
| [`crates/aozora-query`](./crates/aozora-query) | Tree-sitter-style pattern DSL (`SyntaxKind` + capture) for queries over the CST. |
| [`crates/aozora-pandoc`](./crates/aozora-pandoc) | Pandoc AST projection (`Tree` → `pandoc_ast::Pandoc`); unlocks 50+ output formats via Pandoc writers. |
| [`crates/aozora-cli`](./crates/aozora-cli) | `aozora` binary: `check` / `lint` / `fmt` / `render` / `inspect` / `pandoc` / `explain` / `spec` (kinds / schema / slugs) / `completions`. |
| [`crates/aozora-fmt`](./crates/aozora-fmt) | Standalone idempotent formatter (`aozora-fmt` binary) — the engine behind `aozora fmt`, shared with editors and CI. |
| [`crates/aozora-lsp`](./crates/aozora-lsp) | Language server (tower-lsp / stdio): diagnostics, formatting, hover, completion, semantic tokens. Bundled in the VS Code extension. |
| [`crates/tree-sitter-aozora`](./crates/tree-sitter-aozora) | Tree-sitter grammar — the syntactic skeleton `aozora-lsp` queries on every keystroke. |
| [`crates/aozora-wasm`](./crates/aozora-wasm) | `wasm32-unknown-unknown` target for `wasm-pack build --target web`. |
| [`crates/aozora-ffi`](./crates/aozora-ffi) | C ABI driver (opaque handle, JSON-encoded structured data). |
| [`crates/aozora-extism`](./crates/aozora-extism) | Extism (WASM) plugin driver — one portable `aozora.wasm` for polyglot host SDKs (Go / Java / PHP / Ruby / …). The breadth strategy for new languages (ADR-0006). |
| [`crates/aozora-go`](./crates/aozora-go) | Go host SDK over `aozora.wasm` via pure-Go wazero (no cgo). A Go module, not a cargo crate (in `exclude`). |
| [`crates/aozora-py`](./crates/aozora-py) | PyO3 bindings, distributed via `maturin`. |
| [`crates/aozora-bench`](./crates/aozora-bench) | Criterion + corpus-driven probes (PGO profile source). |
| [`crates/aozora-conformance`](./crates/aozora-conformance) | WPT-style conformance fixture runner (golden HTML / serialize / diagnostics / JSON across 60 fixtures). |
| [`crates/aozora-corpus`](./crates/aozora-corpus) | Corpus source abstraction for sweep tests (dev-only, set `AOZORA_CORPUS_ROOT`). |
| [`crates/aozora-proptest`](./crates/aozora-proptest) | Shared proptest strategies (`aozora_fragment` / `pathological_aozora` / `unicode_adversarial` and friends; dev-only). |
| [`crates/aozora-buildstamp`](./crates/aozora-buildstamp) | Compile-time, channel-aware build-version stamp for the binaries (`aozora` CLI / `aozora-lsp`). |
| [`crates/aozora-trace`](./crates/aozora-trace) | DWARF symbolicator for samply traces. |
| [`crates/aozora-xtask`](./crates/aozora-xtask) | Repo automation (samply wrapper, trace analysis, corpus pack/unpack, schema dumps). |

See the [Architecture chapter](https://p4suta.github.io/aozora/arch/pipeline.html)
of the handbook for the layered design, the owned AST, the
SIMD scanner backends, and the dependency graph between these
crates.

## Development

Everything runs inside Docker — the host toolchain is never invoked.
Bring up the dev image once, then drive every operation through `just`:

```sh
just                # list targets
just build          # cargo build --workspace --all-targets
just test           # cargo nextest run --workspace
just prop           # property-based sweep (128 cases per block)
just lint           # fmt + clippy pedantic+nursery + typos + strict-code
just deny           # cargo-deny licenses + advisories + bans
just coverage       # cargo llvm-cov region coverage
just ci             # full CI replica
just book-build     # render the mdbook handbook
just book-serve     # live-preview the handbook at localhost:3000
```

Use `just run` to invoke the CLI inside the container:

```sh
just run check FILE.txt
just run render -E sjis FILE.txt > out.html
```

See [`CONTRIBUTING.md`](./CONTRIBUTING.md) for the contribution flow,
testing strategy, and lint policy.

## Documentation

- 📚 [**Handbook**](https://p4suta.github.io/aozora/) — the mdbook
  site: notation reference, architecture (owned AST,
  SIMD scanner backends, encoding), bindings (Rust / WASM / C ABI /
  Python), performance (samply / bench / corpus sweep), CLI / API /
  env reference, and the contributor guide.
- 📖 [**API reference (rustdoc)**](https://p4suta.github.io/aozora/api/aozora/)
  — auto-deployed alongside the handbook.
- [`CONTRIBUTING.md`](./CONTRIBUTING.md) — dev setup, TDD flow,
  PR rules.
- [`SECURITY.md`](./SECURITY.md) — vulnerability disclosure.
- [`CHANGELOG.md`](./CHANGELOG.md) — release history.

## Related projects

| Repo | What it is |
|---|---|
| [`P4suta/aozora-flavored-markdown`](https://github.com/P4suta/aozora-flavored-markdown) | CommonMark + GFM + 青空文庫記法 integrated Markdown dialect, built on top of this parser. |

## License

Dual-licensed under [Apache-2.0](./LICENSE-APACHE) OR [MIT](./LICENSE-MIT)
at your option, matching Rust community convention. See
[`NOTICE`](./NOTICE) for third-party attribution (Aozora Bunko spec
snapshots and public-domain sample works used in tests).
