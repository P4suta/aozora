# aozora-fmt

Builds `aozora-fmt`, the canonical formatter for aozora documents
(`.afm` / `.aozora` / `.aozora.txt`). It is `parse ∘ to_source`, so
formatting twice is byte-identical to formatting once.

The same formatting ships as `aozora fmt` in the main
[`aozora`](https://github.com/P4suta/aozora) CLI, and as
`textDocument/formatting` in `aozora-lsp`. All three call one function,
so an editor and a CI gate cannot disagree.

## Install

```sh
cargo install --git https://github.com/P4suta/aozora --locked aozora-fmt
```

## Use

```sh
aozora-fmt doc.afm            # canonical form to stdout
aozora-fmt --check docs/      # rustfmt-style gate; exit 1 if it would reformat
aozora-fmt --write doc.afm    # rewrite in place
```

Paths may be files or directories; `-` reads stdin. `aozora-fmt --help`
has the rest.

Exit codes: `0` clean, `1` `--check` would reformat, `2` any other error.

## Library

```rust
let canonical = aozora_fmt::format_source("｜青梅《おうめ》へ");
assert_eq!(canonical, aozora_fmt::format_source(&canonical));
```

Part of the [aozora](https://github.com/P4suta/aozora) workspace.
Dual-licensed Apache-2.0 OR MIT.
