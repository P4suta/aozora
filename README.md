# aozora

<p align="center">
  <a href="https://github.com/P4suta/aozora/actions/workflows/ci.yml"><img alt="ci" src="https://github.com/P4suta/aozora/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/P4suta/aozora/actions/workflows/docs.yml"><img alt="docs deploy" src="https://github.com/P4suta/aozora/actions/workflows/docs.yml/badge.svg"></a>
  <a href="https://github.com/P4suta/aozora/releases/latest"><img alt="latest release" src="https://img.shields.io/github/v/release/P4suta/aozora?display_name=tag&sort=semver"></a>
  <a href="./LICENSE-APACHE"><img alt="license" src="https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue"></a>
  <a href="./rust-toolchain.toml"><img alt="msrv" src="https://img.shields.io/badge/rust-1.95%2B-orange"></a>
</p>

<p align="center">
  📖 <a href="https://p4suta.github.io/aozora/"><strong>API reference (rustdoc)</strong></a>
  · 📦 <a href="https://github.com/P4suta/aozora/releases"><strong>Releases &amp; binaries</strong></a>
  · 🧱 <a href="./docs/adr/"><strong>ADRs</strong></a>
</p>

Pure-functional Rust parser for **青空文庫記法** (Aozora Bunko notation):
ruby (`｜青梅《おうめ》`), bouten (`［＃「X」に傍点］`), 縦中横, 外字
references (`※［＃…、第3水準1-85-54］`), kunten / kaeriten,
indent / align-end containers (`［＃ここから2字下げ］… ［＃ここで字下げ終わり］`),
page and section breaks.

The parser is **comrak-free, CommonMark-free, and Markdown-free** —
this repo deals only with the 青空文庫 notation itself. The
CommonMark+GFM Markdown dialect that layers on top of it lives in
[`P4suta/afm`](https://github.com/P4suta/afm); the authoring
environment (formatter, LSP, VS Code extension) lives in
[`P4suta/aozora-tools`](https://github.com/P4suta/aozora-tools).

## Crate layout

| Crate | Purpose |
|---|---|
| `crates/aozora` | Top-level facade — `Document::parse() → AozoraTree<'_>`, structured `Diagnostic`s, `PairLink` side-table, `SLUGS` catalogue, `canonicalise_slug`, `node_at_source`. The single front door. |
| `crates/aozora-syntax` | AST types (`AozoraNode` borrowed-arena variants, accent table, `ContainerKind`, `Bouten*`). |
| `crates/aozora-lex` | Fused streaming lexer — emits the `BorrowedLexOutput` consumed by the renderer. |
| `crates/aozora-lexer` | Original 7-phase classifier (still used for legacy paths and as the spec backbone). |
| `crates/aozora-render` | HTML and serialise renderers — `html::render_to_string`, `serialize::serialize` (single O(n) walker). |
| `crates/aozora-encoding` | Shift_JIS decoding + 外字 lookup (compile-time PHF table, JIS X 0213 + UCS resolution). |
| `crates/aozora-spec` | Spec slugs, kaeriten / bouten / indent canonical form, accent digraph table. |
| `crates/aozora-scan` | SIMD-friendly multi-pattern scanner backends (memchr / regex-automata / teddy / structural-bitmap; chosen per ADR-0015 bake-off). |
| `crates/aozora-veb` | Sorted slot-map (van Emde Boas-style) for placeholder registry lookup. |
| `crates/aozora-corpus` | Corpus source abstraction for sweep tests (dev-only, set `AOZORA_CORPUS_ROOT`). |
| `crates/aozora-test-utils` | Shared proptest strategies (dev-only). |
| `crates/aozora-cli` | `aozora` binary: `check` / `fmt` / `render`. |
| `crates/aozora-bench` | criterion + corpus-driven bench harness (PGO profile source). |
| `crates/aozora-trace` | DWARF symbolicator for samply traces of the bench binary. |
| `crates/aozora-ffi` | C ABI bindings (work in progress). |
| `crates/aozora-wasm` | `wasm32-unknown-unknown` target for `wasm-pack build --target web`. |
| `crates/aozora-py` | PyO3 bindings (work in progress). |
| `crates/aozora-xtask` | Repo automation (corpus pack/unpack, sanitizers harness). |

## Quickstart

Everything runs inside Docker — host toolchain is never invoked. Bring
up the dev image once, then drive every operation through `just`:

```sh
just                    # list targets
just build              # cargo build --workspace
just test               # cargo nextest run --workspace
just spec-aozora        # hand-written annotation fixtures
just spec-golden-56656  # 罪と罰 (card 56656) Tier-A acceptance gate
just lint               # fmt + clippy + typos + strict-code purity check
just deny               # cargo-deny licenses + advisories + bans
just audit              # cargo-audit RustSec advisory scan
just ci                 # full CI replica (gates on all of the above)
```

Use `just run` to invoke the CLI:

```sh
just run check FILE.txt           # lex + report diagnostics
just run fmt --check FILE.txt     # round-trip parse ∘ serialize check
just run render FILE.txt          # render to HTML on stdout
just run check -E sjis FILE.txt   # Shift_JIS source from Aozora Bunko
```

## Public API contract

The library surface is the top-level [`aozora`](./crates/aozora) crate;
both `aozora-tools` and `afm` consume only its re-exports.

```rust
use aozora::{Document, AozoraTree, Diagnostic, SLUGS, canonicalise_slug};
use aozora::html;
use aozora_encoding::gaiji;

let arena = bumpalo::Bump::new();
let doc = Document::new("｜青梅《おうめ》", &arena);
let tree: AozoraTree<'_> = doc.parse();
let diagnostics: &[Diagnostic] = tree.diagnostics();
let html_string: String = html::render_to_string("｜青梅《おうめ》");
```

ADR-0001 (zero parser hooks) is the architectural thesis: every Aozora
recogniser is a pure function in the lexer's 7-phase pipeline. The
renderer only consumes the lexer's output. ADR-0010 covers the
borrowed-arena AST (`AozoraNode<'src>` shares lifetimes with the source
buffer), and ADRs 0014–0020 document the perf/algorithmic milestones
since v0.1.0 — see [`docs/adr/`](./docs/adr/) for the full chain.

## Three-layer ecosystem

```
┌─────────────────────────────────────────┐
│ aozora-tools  (authoring environment)    │
│   crates/aozora-fmt    formatter CLI     │
│   crates/aozora-lsp    LSP server        │
│   editors/vscode/      TS client         │
└──────────────┬──────────────────────────┘
               │ depends on aozora via git tag
               ▼
┌─────────────────────────────────────────┐
│ afm  (CommonMark + GFM + aozora dialect) │
│   crates/afm-markdown  Markdown layer    │
│   crates/afm-cli       afm render/check  │
│   upstream/comrak/     vendored fork     │
└──────────────┬──────────────────────────┘
               │ depends on aozora via git tag
               ▼
┌─────────────────────────────────────────┐
│ aozora  (this repo — pure 青空文庫記法)   │
└─────────────────────────────────────────┘
```

See [`docs/adr/0006-authoring-tools-live-in-sibling-repositories.md`](docs/adr/0006-authoring-tools-live-in-sibling-repositories.md)
for the split rationale.

## Sibling repositories

| Repo | What it is |
|---|---|
| [`P4suta/afm`](https://github.com/P4suta/afm) | CommonMark + GFM + 青空文庫記法 integrated Markdown dialect, built on top of this parser. |
| [`P4suta/aozora-tools`](https://github.com/P4suta/aozora-tools) | Authoring tools: `aozora-fmt`, `aozora-lsp` (LSP server), tree-sitter grammar, VS Code extension. |

## Install

Pre-built `aozora` CLI binaries for **Linux x86_64**, **macOS arm64**,
and **Windows x86_64** are attached to every GitHub Release — see
[the releases page](https://github.com/P4suta/aozora/releases) and pick
a `aozora-vX.Y.Z-<target>.{tar.gz,zip}`. SHA256 sums are published as
`SHA256SUMS` next to the archives.

Or build from source:

```sh
cargo install --git https://github.com/P4suta/aozora --tag v0.2.4 --locked aozora-cli
```

## Use as a Rust library

In your `Cargo.toml`:

```toml
[dependencies]
aozora          = { version = "0.2.4", git = "https://github.com/P4suta/aozora.git", tag = "v0.2.4" }
aozora-encoding = { version = "0.2.4", git = "https://github.com/P4suta/aozora.git", tag = "v0.2.4" }
```

(crates.io publication is on the roadmap once the public API stabilises
to the 1.0 contract.)

## Security

Vulnerabilities go through GitHub Security Advisories — see
[`SECURITY.md`](./SECURITY.md) for the disclosure flow.

## License

Dual-licensed under [Apache-2.0](./LICENSE-APACHE) OR [MIT](./LICENSE-MIT)
at your option, matching Rust community convention. See
[`NOTICE`](./NOTICE) for third-party attribution (JIS X 0213 tables,
Aozora Bunko fixtures used in tests).
