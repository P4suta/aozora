# aozora-lsp

A Language Server for
[aozora-flavored-markdown](https://github.com/P4suta/aozora-flavored-markdown)
(`.afm` / `.aozora` / `.aozora.txt`). It speaks LSP on stdio.

## Install

The bundled
[VS Code extension](https://github.com/P4suta/aozora/tree/main/editors/vscode)
ships this binary, so it needs no separate install. For any other
LSP-capable editor:

```sh
cargo install --git https://github.com/P4suta/aozora --locked aozora-lsp
```

Editors may spawn `aozora-lsp --stdio` directly, or `aozora lsp` — a
git-style exec-delegate that finds this binary and forwards every
argument to it, keeping the LSP dependencies out of the `aozora` binary.

Beyond the standard methods it serves two custom requests,
`aozora/renderHtml` and `aozora/gaijiSpans`, both documented at
[docs.rs/aozora-lsp](https://docs.rs/aozora-lsp).

Part of the [aozora](https://github.com/P4suta/aozora) workspace.
Dual-licensed Apache-2.0 OR MIT.
