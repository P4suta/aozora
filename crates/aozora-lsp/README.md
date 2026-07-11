# aozora-lsp

Language Server (tower-lsp) for
[aozora-flavored markdown](https://github.com/P4suta/aozora)
(`.afm` / `.aozora` / `.aozora.txt`).

`aozora-lsp` speaks LSP on stdio. Pair it with the bundled
[VS Code extension](https://github.com/P4suta/aozora/tree/main/editors/vscode)
or any LSP-capable editor (Neovim, Helix, Emacs, Zed, …).

## Capabilities

| LSP method                       | Behaviour |
|----------------------------------|-----------|
| `textDocument/publishDiagnostics`| Every `aozora::Diagnostic` mapped to LSP `Diagnostic` (UTF-16 column-aware). |
| `textDocument/formatting`        | `parse ∘ serialize` (via `aozora_fmt::format_source`); single document-replace `TextEdit`. |
| `textDocument/hover`             | Cursor inside `※［＃…］` resolves the gaiji glyph + description via `aozora_encoding::gaiji`. |
| `textDocument/linkedEditingRange`| Paired delimiters auto-rename together: `［` ↔ `］`, `《` ↔ `》`, `「` ↔ `」`, … |
| `textDocument/completion`        | Slug catalogue from `aozora::SLUGS`, parametric snippets, paired-container partner auto-insert. |
| `textDocument/foldingRange`      | Section / paragraph folding. |
| `textDocument/documentSymbol`    | Heading outline. |
| `textDocument/semanticTokens`    | Syntax-aware highlighting derived from the tree-sitter tree. |
| `workspace/executeCommand`       | `aozora.canonicalizeSlug` — canonicalise the slug at a given range. |
| `aozora/renderHtml` *(custom)*   | Returns HTML for the active document. Drives the VS Code preview pane. |
| `aozora/gaijiSpans` *(custom)*   | Every resolvable gaiji in the document with `range` + resolved glyph; the VS Code extension uses it for inline-fold decorations. Any LSP client can consume this request directly for the same data. |

`textDocument/inlayHint` is intentionally **not** advertised — the VS
Code extension already renders the resolved-gaiji glyph through
`aozora/gaijiSpans`-driven decorations, and adding an LSP inlay layer
duplicated the visual. Editors that want the same data consume the
`aozora/gaijiSpans` custom request.

## Architecture (one-liner)

Per-document state is split into a writer-side `Mutex<BufferState>`
(Rope buffer + tree-sitter incremental parse, paragraph-segmented)
and a reader-side `ArcSwap<Snapshot>` for wait-free LSP request
handling. The semantic `aozora` parser is the source of truth for
formatting, diagnostics, and HTML rendering; the tree-sitter tree
backs the high-frequency syntactic queries (hover, inlay,
completion, codeAction).

## Documentation

The canonical documentation for `aozora-lsp` is the
[handbook](https://p4suta.github.io/aozora/lsp/overview.html):

- [Overview](https://p4suta.github.io/aozora/lsp/overview.html)
- [State model](https://p4suta.github.io/aozora/lsp/state-model.html)
- [Standard LSP capabilities](https://p4suta.github.io/aozora/lsp/capabilities.html)
- [Custom protocol extensions](https://p4suta.github.io/aozora/lsp/extensions.html)
- [Diagnostics catalogue](https://p4suta.github.io/aozora/lsp/diagnostics.html)

## Run

```sh
# stdio LSP server (default)
cargo run --bin aozora-lsp

# Tracing on stderr (set RUST_LOG to your taste)
RUST_LOG=aozora_lsp=debug cargo run --bin aozora-lsp
```

## Test surface

```sh
cargo test  -p aozora-lsp --all-targets         # unit + integration
cargo test  -p aozora-lsp --features shuttle-tests   shuttle_doc_state   # randomized-schedule
cargo bench -p aozora-lsp --bench burst         # apply-edits p99 / max
```

The `tests/guardian.rs` 金庫番 suite is the panic-resistance,
idempotence, and concurrency invariant gate. The `burst` bench
(driven by `samples/bouten.afm`) is what the criterion-based
PR diff workflow (`.github/workflows/bench-diff.yml`) gates on.

Profiling pipeline (samply) is documented in the
[handbook's Profiling chapter](https://p4suta.github.io/aozora/perf/samply.html).

## Install

```sh
cargo install --git https://github.com/P4suta/aozora --locked aozora-lsp
```

The VS Code extension ships the matching `aozora-lsp` binary baked in, so no
separate install is needed there; for other editors, `cargo install` as above.

## Repository

Part of the [aozora](https://github.com/P4suta/aozora)
workspace. See the
[workspace README](https://github.com/P4suta/aozora#readme) for the
full picture.
