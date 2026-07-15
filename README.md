# aozora

<p align="center">
  <a href="https://github.com/P4suta/aozora/actions/workflows/ci.yml"><img alt="ci" src="https://github.com/P4suta/aozora/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://crates.io/crates/aozora"><img alt="crates.io" src="https://img.shields.io/crates/v/aozora.svg"></a>
  <a href="https://docs.rs/aozora"><img alt="docs.rs" src="https://img.shields.io/docsrs/aozora"></a>
  <a href="./LICENSE-APACHE"><img alt="license" src="https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue"></a>
  <a href="https://p4suta.github.io/aozora/contrib/msrv.html"><img alt="msrv" src="https://img.shields.io/crates/msrv/aozora"></a>
  <a href="https://scorecard.dev/viewer/?uri=github.com/P4suta/aozora"><img alt="OpenSSF Scorecard" src="https://api.securityscorecards.dev/projects/github.com/P4suta/aozora/badge"></a>
</p>

A parser for **青空文庫記法** (Aozora Bunko notation): ruby (`｜青梅《おうめ》`),
bouten (`［＃「X」に傍点］`), 縦中横, 外字 (`※［＃…、第3水準1-85-54］`),
kunten / kaeriten, indent containers, and page breaks. Reads Shift_JIS or UTF-8;
emits HTML, JSON, canonical source, or — via Pandoc — EPUB, LaTeX, DOCX and the
rest.

Usable from Rust, the command line, JavaScript, Python, Go, C, and any
[Extism](https://extism.org) host.

## Install

```sh
cargo install aozora-cli --locked   # command line
cargo add aozora                    # Rust library
npm install aozora-wasm             # JavaScript
pip install aozora-py               # Python
```

Pre-built CLI binaries for Linux, macOS, and Windows are on the
[releases page](https://github.com/P4suta/aozora/releases).

## Use

```sh
aozora check FILE.txt      # report diagnostics
aozora render FILE.txt     # HTML on stdout
aozora fmt FILE.txt        # rewrite to canonical form
```

Every subcommand reads stdin when given `-` or no path. `-E sjis` reads
Shift_JIS.

```rust
use aozora::Document;

let doc = Document::new("｜青梅《おうめ》".to_owned());
let tree = doc.parse();

let html = tree.to_html();
let diagnostics = tree.diagnostics();
```

## Documentation

- [Playground](https://p4suta.github.io/aozora/playground/) — try it in the browser
- [Handbook](https://p4suta.github.io/aozora/) — notation reference, recipes, bindings
- [API reference](https://docs.rs/aozora)
- [Notation specification](https://p4suta.github.io/aozora-notation-spec/) — the normative spec this parser is tested against

## Contributing

[`CONTRIBUTING.md`](./CONTRIBUTING.md) · [`SECURITY.md`](./SECURITY.md) ·
[`CHANGELOG.md`](./CHANGELOG.md)

[`aozora-flavored-markdown`](https://github.com/P4suta/aozora-flavored-markdown)
builds a CommonMark + GFM + 青空文庫記法 dialect on top of this parser.

## License

Apache-2.0 OR MIT, at your option. [`NOTICE`](./NOTICE) lists third-party
attribution.
