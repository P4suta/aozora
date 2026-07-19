# aozora

A parser for **青空文庫記法** (Aozora Bunko notation). Reads Shift_JIS or
UTF-8; emits HTML, JSON, or canonical source.

```rust
use aozora::Document;

let doc = Document::new("｜青梅《おうめ》".to_owned());
let snapshot = doc.snapshot();

let html = snapshot.to_html();
let diagnostics = snapshot.diagnostics();
```

Depend on this crate alone for parsing, diagnostics, rendering, and incremental
editing.

## Documentation

- [API reference](https://docs.rs/aozora) — including which features gate what
- [Examples](https://github.com/P4suta/aozora/tree/main/crates/aozora/examples)
- [Notation specification](https://p4suta.github.io/aozora-notation-spec/)
- [Playground](https://p4suta.github.io/aozora/playground/) — try it in the browser

## License

Apache-2.0 OR MIT, at your option.
