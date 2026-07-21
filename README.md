# aozora

<p align="center">
  <a href="https://github.com/P4suta/aozora/actions/workflows/ci.yml"><img alt="ci" src="https://github.com/P4suta/aozora/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://crates.io/crates/aozora"><img alt="crates.io" src="https://img.shields.io/crates/v/aozora.svg"></a>
  <a href="https://docs.rs/aozora"><img alt="docs.rs" src="https://img.shields.io/docsrs/aozora"></a>
  <a href="./LICENSE-APACHE"><img alt="license" src="https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue"></a>
  <a href="https://github.com/P4suta/aozora/blob/main/docs/contrib/msrv.md"><img alt="msrv" src="https://img.shields.io/crates/msrv/aozora"></a>
  <a href="https://scorecard.dev/viewer/?uri=github.com/P4suta/aozora"><img alt="OpenSSF Scorecard" src="https://api.securityscorecards.dev/projects/github.com/P4suta/aozora/badge"></a>
</p>

A parser for **青空文庫記法** (Aozora Bunko notation). Reads Shift_JIS or
UTF-8; emits HTML, JSON, or canonical source — and, via Pandoc, EPUB,
LaTeX, DOCX and the rest.

## Install

```sh
cargo install aozora-cli --locked   # command line
cargo add aozora                    # Rust library
npm install aozora-wasm             # JavaScript
pip install aozora                  # Python
```

Pre-built CLI binaries are on the
[releases page](https://github.com/P4suta/aozora/releases). The Go SDK
ships there as a tarball (`aozora-go.tar.gz`);
[`crates/aozora-go`](./crates/aozora-go/README.md) covers its
`replace`-directive install.

## Use

```sh
aozora check FILE.txt      # report diagnostics
aozora render FILE.txt     # HTML on stdout
aozora fmt FILE.txt        # rewrite to canonical form
```

Every subcommand reads stdin when given `-` or no path. Encoding is
detected; `-E sjis` forces it. `aozora --help` lists the rest.

```rust
use aozora::parse;

let doc = parse("｜青梅《おうめ》").expect("source fits parser limits");
let snapshot = doc.snapshot();

let html = snapshot.to_html();
let diagnostics = snapshot.diagnostics();
```

## Documentation

- [Playground](https://p4suta.github.io/aozora/playground/) — try it in the browser
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
