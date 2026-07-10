# tree-sitter-aozora

Tree-sitter grammar for
[aozora-flavored markdown](https://github.com/P4suta/aozora) —
the syntactic skeleton `aozora-lsp` queries on every keystroke
to keep hover / inlay / completion / codeAction request latency
size-independent.

The semantic Rust parser (`aozora` in the sibling repo) stays the
source of truth for formatting, HTML rendering, and diagnostics —
operations where the tree-sitter syntax tree is too thin. The LSP
runs both parsers in parallel; this grammar is what makes the
high-frequency LSP handlers responsive on 100 k+ documents.

## Coverage (Stage 1)

| Node            | Source pattern                          |
|-----------------|------------------------------------------|
| `gaiji`         | `※［＃…］`                                |
| `slug`          | `［＃…］`                                 |
| `explicit_ruby` | `｜base《reading》`                       |
| `implicit_ruby` | `kanji-run《reading》` (kanji autodetect)  |
| `text`          | catch-all run                            |
| `newline`       | line break                               |

Out of scope (Stage 2+): `〔…〕` accent decomposition, `《《…》》`
double-bracket emphasis, kaeriten, 縦中横.

See `grammar.js` for the full disambiguation rules.

## Build

`grammar.js` is the source of truth. `tree-sitter generate` compiles it into
the committed `src/parser.c` (+ `src/grammar.json` / `src/node-types.json`),
and `build.rs` turns `parser.c` into the static parser the Rust binding
links against — so downstream consumers only need a C toolchain, not the
tree-sitter CLI.

Regeneration is drift-gated: `xtask conformance grammar --check` (run by
`just drift-gate` in CI) fails if the committed artefacts have drifted from a
fresh generate of `grammar.js`. The dev image pins the tree-sitter CLI to the
same version as the `tree-sitter` runtime crate, and the CLI embeds its own JS
engine to evaluate `grammar.js`, so regeneration needs no `node`.

```sh
# Regenerate the committed artefacts from grammar.js (writes src/parser.c,
# src/grammar.json, src/node-types.json), then commit the diff.
just grammar

# Verify the committed artefacts match a fresh generate (the drift gate).
just grammar-check

# Test the grammar (Rust integration tests in bindings/rust/lib.rs)
cargo test -p tree-sitter-aozora
```

## Rust binding

```toml
[dependencies]
tree-sitter         = "0.26"
tree-sitter-aozora  = { path = "crates/tree-sitter-aozora" }
```

```rust
use tree_sitter::Parser;
use tree_sitter_aozora::LANGUAGE;

let mut parser = Parser::new();
parser.set_language(&LANGUAGE.into()).unwrap();
let tree = parser.parse(source, None).unwrap();
```

## Repository

Part of the [aozora](https://github.com/P4suta/aozora)
workspace.
