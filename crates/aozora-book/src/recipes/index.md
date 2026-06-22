# Recipes

Task-shaped, copy-paste answers to "how do I *do* X with aozora?".
Each recipe is a single problem stated in one sentence, the minimal
correct code to solve it, the output you should expect, and a jump
list to the deeper chapters.

The Rust snippets use the umbrella [`aozora`](../bindings/rust.md)
crate and nothing else — downstream consumers depend on `aozora`
alone, never the internal build-block crates. The shell snippets use
the [`aozora`](../ref/cli.md) binary. If you have not yet got either
in scope, start at [Install](../getting-started/install.md), then the
[Library](../getting-started/library.md) or
[CLI](../getting-started/cli.md) quickstart.

Each recipe that has a Rust solution maps to a runnable example under
`crates/aozora/examples/`, so you can read the whole program and run
it rather than reassembling fragments. Where that applies the recipe
says so — e.g. *run with `just example walk_ast`*.

## The recipes

| I want to…                                            | Recipe                                            |
| ----------------------------------------------------- | ------------------------------------------------- |
| Pull every ruby base + reading pair out of a document | [Extract ruby pairs](extract-ruby.md)             |
| Get diagnostics as machine-readable JSON              | [Diagnostics as JSON](diagnostics-json.md)        |
| Walk the parsed tree node by node                     | [Walk the AST](walk-ast.md)                       |
| Show a notation verbatim in a host literal context    | [Notations in host literal contexts](literal-contexts.md) |
| Parse a Shift_JIS file and resolve 外字               | [Shift_JIS & gaiji](sjis-gaiji.md)                |
| Convert to EPUB / LaTeX / DOCX                        | [EPUB via Pandoc](epub-pandoc.md)                 |
| Check that a file is already canonical                | [Round-trip & fmt --check](round-trip.md)         |
| Call aozora from Go / Java / Python / JS              | [Call from another language](polyglot.md)         |
| Check or render a whole directory of files            | [Batch many files](batch.md)                      |

## The example programs

The recipes mirror these runnable examples (authored under
`crates/aozora/examples/`); each is launched with `just example <name>`:

| Example      | Mirrors                                       |
| ------------ | --------------------------------------------- |
| `hello`      | The six-line render in the [Library quickstart](../getting-started/library.md) |
| `walk_ast`   | [Walk the AST](walk-ast.md), [Extract ruby pairs](extract-ruby.md) |
| `diagnostics`| [Diagnostics as JSON](diagnostics-json.md)    |
| `round_trip` | [Round-trip & fmt --check](round-trip.md)     |
| `sjis`       | [Shift_JIS & gaiji](sjis-gaiji.md)            |

## See also

- [Library Quickstart](../getting-started/library.md) — the lifetime
  model and the core `Document` → `Tree` flow every recipe
  assumes.
- [Choosing a binding](../bindings/choosing.md) — picking the surface
  (Rust / CLI / wasm / Python / Go / Extism) before you start.
- [Node reference](../nodes/index.md) — what each AST node represents.
- [JSON output](../json/overview.md) — the JSON envelope the
  `aozora::json` serialisers emit.
