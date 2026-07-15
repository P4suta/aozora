# aozora

A parser for **青空文庫記法** (Aozora Bunko notation). Reads Shift_JIS or
UTF-8; emits HTML, JSON, or canonical source.

```rust
use aozora::Document;

let doc = Document::new("｜青梅《おうめ》".to_owned());
let tree = doc.parse();

let html = tree.to_html();
let diagnostics = tree.diagnostics();
```

Depend on this crate alone. The build-block crates it sits over carry no
stability contract of their own.

## Documentation

- [API reference](https://docs.rs/aozora) — including which features gate what
- [Examples](https://github.com/P4suta/aozora/tree/main/crates/aozora/examples)
- [Notation specification](https://p4suta.github.io/aozora-notation-spec/)
- [Playground](https://p4suta.github.io/aozora/playground/) — try it in the browser

## License

Apache-2.0 OR MIT, at your option.
