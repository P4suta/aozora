# aozora

<p align="center">
  <a href="https://crates.io/crates/aozora"><img alt="crates.io" src="https://img.shields.io/crates/v/aozora.svg"></a>
  <a href="https://docs.rs/aozora"><img alt="docs.rs" src="https://img.shields.io/docsrs/aozora"></a>
  <a href="https://github.com/P4suta/aozora/blob/main/LICENSE-APACHE"><img alt="license" src="https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue"></a>
  <a href="https://p4suta.github.io/aozora/contrib/msrv.html"><img alt="msrv" src="https://img.shields.io/crates/msrv/aozora"></a>
</p>

A parser for **青空文庫記法** (Aozora Bunko notation): ruby (`｜青梅《おうめ》`),
bouten (`［＃「X」に傍点］`), 縦中横, 外字 (`※［＃…、第3水準1-85-54］`),
kunten / kaeriten, indent containers, and page breaks. Reads Shift_JIS or UTF-8;
emits HTML, JSON, or canonical source.

```rust
use aozora::Document;

let doc = Document::new("｜青梅《おうめ》".to_owned());
let tree = doc.parse();

let html = tree.to_html();
let diagnostics = tree.diagnostics();
```

```sh
cargo add aozora
```

Shift_JIS decoding, the CST, the query DSL, and JSON Schema export are behind
cargo features — see the [feature list](https://docs.rs/aozora).

**Depend on this crate alone.** It is the stable seam over the build-block
crates (`aozora-syntax`, `aozora-pipeline`, …), which are published only so this
one can depend on them and carry no API-stability contract of their own.

## Documentation

- [API reference](https://docs.rs/aozora)
- [Handbook](https://p4suta.github.io/aozora/) — notation reference, recipes, other bindings
- [Playground](https://p4suta.github.io/aozora/playground/) — try it in the browser

For other languages there is a [CLI](https://crates.io/crates/aozora-cli),
[npm](https://www.npmjs.com/package/aozora-wasm), [PyPI](https://pypi.org/project/aozora-py/),
Go, a C ABI, and an [Extism](https://extism.org) plugin — all one parser, so the
output is byte-identical whichever you pick.

## License

Apache-2.0 OR MIT, at your option.
