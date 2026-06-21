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

`parser.c` is committed (regenerated via `tree-sitter generate`
from `grammar.js`). Downstream consumers only need a C
toolchain — `node` and the tree-sitter CLI are required only to
regenerate.

```sh
# Regenerate parser.c from grammar.js (writes src/parser.c)
npx tree-sitter generate

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

Part of the [aozora-tools](https://github.com/P4suta/aozora-tools)
workspace.
