# aozora

<p align="center">
  <a href="https://github.com/P4suta/aozora/actions/workflows/ci.yml"><img alt="ci" src="https://github.com/P4suta/aozora/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/P4suta/aozora/actions/workflows/docs.yml"><img alt="docs deploy" src="https://github.com/P4suta/aozora/actions/workflows/docs.yml/badge.svg"></a>
  <a href="https://github.com/P4suta/aozora/releases/latest"><img alt="latest release" src="https://img.shields.io/github/v/release/P4suta/aozora?display_name=tag&sort=semver"></a>
  <a href="./LICENSE-APACHE"><img alt="license" src="https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue"></a>
  <a href="./rust-toolchain.toml"><img alt="msrv" src="https://img.shields.io/badge/rust-1.95-orange"></a>
</p>

<p align="center">
  📖 <a href="https://p4suta.github.io/aozora/"><strong>API reference (rustdoc)</strong></a>
  · 📦 <a href="https://github.com/P4suta/aozora/releases"><strong>Releases &amp; binaries</strong></a>
  · 🏛️ <a href="./docs/ARCHITECTURE.md"><strong>Architecture</strong></a>
  · 🚀 <a href="./docs/USAGE.md"><strong>Usage guide</strong></a>
</p>

Pure-functional Rust parser for **青空文庫記法** (Aozora Bunko notation):
ruby (`｜青梅《おうめ》`), bouten (`［＃「X」に傍点］`), 縦中横, 外字
references (`※［＃…、第3水準1-85-54］`), kunten / kaeriten,
indent / align-end containers (`［＃ここから2字下げ］… ［＃ここで字下げ終わり］`),
and page / section breaks.

The parser is **CommonMark-free, Markdown-free** — this repository deals
only with the 青空文庫 notation itself. The renderer emits semantic HTML5;
the lexer reports structured diagnostics; the AST is a borrowed-arena
tree that can be walked in O(n) without copying source bytes.

## Installation

### Pre-built CLI

Pre-built `aozora` CLI binaries for **Linux x86_64**, **macOS arm64**,
and **Windows x86_64** are attached to every GitHub Release —
[the releases page](https://github.com/P4suta/aozora/releases) carries
`aozora-vX.Y.Z-<target>.{tar.gz,zip}` archives with `SHA256SUMS`.

### Build from source

```sh
cargo install --git https://github.com/P4suta/aozora --tag v0.2.5 --locked aozora-cli
```

### As a Rust library

```toml
[dependencies]
aozora          = { git = "https://github.com/P4suta/aozora.git", tag = "v0.2.5" }
aozora-encoding = { git = "https://github.com/P4suta/aozora.git", tag = "v0.2.5" }
```

(crates.io publication tracks the 1.0 API freeze.)

For WASM / C ABI / Python bindings see [`docs/USAGE.md`](./docs/USAGE.md).

## Quickstart

```rust
use aozora::Document;

let source = "｜青梅《おうめ》".to_owned();
let doc = Document::new(source);
let tree = doc.parse();

let html: String = tree.to_html();
let canonical: String = tree.serialize();
let diagnostics = tree.diagnostics();

assert_eq!(canonical, "｜青梅《おうめ》");
```

`Document` owns a [`bumpalo`](https://docs.rs/bumpalo) arena; `tree`
borrows from it for the lifetime of the `Document`. Dropping the
`Document` releases every node in a single `Bump::reset` step.

## CLI

```sh
aozora check FILE.txt           # lex + report diagnostics
aozora fmt --check FILE.txt     # round-trip parse ∘ serialize check
aozora render FILE.txt          # render to HTML on stdout
aozora check -E sjis FILE.txt   # Shift_JIS source from Aozora Bunko
```

All subcommands accept `-` (or no path argument) to read from stdin.
See [`docs/USAGE.md`](./docs/USAGE.md) for full subcommand reference.

## Crate layout

| Crate | Purpose |
|---|---|
| [`crates/aozora`](./crates/aozora) | Top-level facade. `Document::parse() → AozoraTree<'_>`, structured `Diagnostic`s, `SLUGS` catalogue, `canonicalise_slug`. The single front door. |
| [`crates/aozora-spec`](./crates/aozora-spec) | Single source of truth for shared types: `Span`, `TriggerKind`, `PairKind`, `Diagnostic`, PUA sentinel codepoints. No internal dependency. |
| [`crates/aozora-syntax`](./crates/aozora-syntax) | AST types (`AozoraNode` borrowed-arena variants, `ContainerKind`, `BoutenKind`, `Indent`). |
| [`crates/aozora-encoding`](./crates/aozora-encoding) | Shift_JIS decoding + 外字 lookup (compile-time PHF, JIS X 0213 + UCS resolution). |
| [`crates/aozora-scan`](./crates/aozora-scan) | SIMD-friendly multi-pattern scanner backends (Teddy, structural-bitmap, DFA fallback). |
| [`crates/aozora-veb`](./crates/aozora-veb) | Eytzinger-layout sorted-set lookup (cache-friendly binary search). |
| [`crates/aozora-lexer`](./crates/aozora-lexer) | 7-phase classifier pipeline. |
| [`crates/aozora-lex`](./crates/aozora-lex) | Fused streaming orchestrator — pure `fn(&str) -> AozoraTree<'_>`. |
| [`crates/aozora-render`](./crates/aozora-render) | HTML and serialise renderers — `html::render_to_string`, `serialize::serialize`. |
| [`crates/aozora-cli`](./crates/aozora-cli) | `aozora` binary: `check` / `fmt` / `render`. |
| [`crates/aozora-wasm`](./crates/aozora-wasm) | `wasm32-unknown-unknown` target for `wasm-pack build --target web`. |
| [`crates/aozora-ffi`](./crates/aozora-ffi) | C ABI driver (opaque handle, JSON-encoded structured data). |
| [`crates/aozora-py`](./crates/aozora-py) | PyO3 bindings, distributed via `maturin`. |
| [`crates/aozora-bench`](./crates/aozora-bench) | Criterion + corpus-driven probes (PGO profile source). |
| [`crates/aozora-corpus`](./crates/aozora-corpus) | Corpus source abstraction for sweep tests (dev-only, set `AOZORA_CORPUS_ROOT`). |
| [`crates/aozora-test-utils`](./crates/aozora-test-utils) | Shared proptest strategies (dev-only). |
| [`crates/aozora-trace`](./crates/aozora-trace) | DWARF symbolicator for samply traces. |
| [`crates/aozora-xtask`](./crates/aozora-xtask) | Repo automation (samply wrapper, trace analysis, corpus pack/unpack). |

See [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md) for the layered
design and the dependency graph between these crates.

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
just coverage       # cargo llvm-cov branch coverage
just ci             # full CI replica
```

Use `just run` to invoke the CLI inside the container:

```sh
just run check FILE.txt
just run render -E sjis FILE.txt > out.html
```

See [`CONTRIBUTING.md`](./CONTRIBUTING.md) for the contribution flow,
testing strategy, and lint policy.

## Documentation

- [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md) — crate layers,
  borrowed-arena AST, SIMD scan strategy, lint and safety policy.
- [`docs/USAGE.md`](./docs/USAGE.md) — CLI / Rust library / WASM /
  C ABI / Python usage, environment variables.
- [`docs/PROFILING.md`](./docs/PROFILING.md) — how to take a samply
  profile, the bench probes, and common pitfalls.
- [`CONTRIBUTING.md`](./CONTRIBUTING.md) — dev setup, TDD flow,
  PR rules.
- [`SECURITY.md`](./SECURITY.md) — vulnerability disclosure.
- [API reference (rustdoc)](https://p4suta.github.io/aozora/) — auto-deployed.

## Related projects

| Repo | What it is |
|---|---|
| [`P4suta/afm`](https://github.com/P4suta/afm) | CommonMark + GFM + 青空文庫記法 integrated Markdown dialect, built on top of this parser. |
| [`P4suta/aozora-tools`](https://github.com/P4suta/aozora-tools) | Authoring tools: formatter, LSP server, tree-sitter grammar, VS Code extension. |

## License

Dual-licensed under [Apache-2.0](./LICENSE-APACHE) OR [MIT](./LICENSE-MIT)
at your option, matching Rust community convention. See
[`NOTICE`](./NOTICE) for third-party attribution (Aozora Bunko spec
snapshots and public-domain sample works used in tests).
