# tree-sitter-aozora

A [Tree-sitter](https://tree-sitter.github.io/) grammar for 青空文庫記法
(Aozora Bunko notation), part of the
[aozora](https://github.com/P4suta/aozora) workspace. The notation
itself is in
[the specification](https://p4suta.github.io/aozora-notation-spec/).

`grammar.js` is the source of truth. Everything under `src/` —
`parser.c`, `grammar.json`, `node-types.json` — is generated from it and
must not be hand-edited.

The Rust API is at
[docs.rs/tree-sitter-aozora](https://docs.rs/tree-sitter-aozora).

Dual-licensed Apache-2.0 OR MIT.
