# aozora-pandoc

Projects a parsed [aozora-flavored markdown](https://github.com/P4suta/aozora)
document into the [Pandoc AST][pandoc-ast]. Once you have Pandoc JSON,
every Pandoc output format (HTML, EPUB, LaTeX/PDF, DOCX, ODT,
MediaWiki, …) is one shell pipe away — the recommended path to convert
Aozora Bunko notation into anything *other* than the built-in HTML
renderer.

Each `aozora::Node` variant lifts to a Pandoc `Span` / `Div` carrying a
stable CSS class (`aozora-ruby`, `aozora-bouten`, …) plus the
structured data as attributes, so downstream filters and stylesheets
can specialise the rendering. See the
[projection-rules table](https://p4suta.github.io/aozora/bindings/pandoc.html)
in the handbook for the full class map.

> **Experimental crate.** The stable, supported surface is the `aozora`
> CLI's [`pandoc` subcommand](https://p4suta.github.io/aozora/ref/cli.html);
> the Rust `to_pandoc` API here can change in any release.

## CLI (the supported path)

```sh
# Pandoc JSON to stdout
aozora pandoc input.txt > out.json

# Or pipe through pandoc directly
aozora pandoc input.txt | pandoc -f json -t html
aozora pandoc input.txt | pandoc -f json -t epub3 -o out.epub

# `--format` is shorthand for the pipe (requires pandoc on PATH)
aozora pandoc input.txt --format html > out.html
aozora pandoc -E sjis legacy.txt --format epub > out.epub
```

For example, `aozora pandoc` on `｜青梅《おうめ》` emits a `Para` whose
ruby lifts to nested `Span`s classed `aozora-ruby` /
`aozora-ruby-base` / `aozora-ruby-reading`.

## Library

```rust
use aozora::Document;
use aozora_pandoc::to_pandoc;

let doc = Document::new("｜青梅《おうめ》");
let owned = doc.lex();
let pandoc = to_pandoc(&owned);
// Serialize to Pandoc JSON for `pandoc -f json -t html`:
let json = serde_json::to_string(&pandoc).expect("serialise pandoc ast");
assert!(json.contains("青梅")); // the ruby base lands in the Pandoc AST
```

## Documentation

- 📖 [API reference (docs.rs)](https://docs.rs/aozora-pandoc)
- 📚 [Pandoc integration chapter](https://p4suta.github.io/aozora/bindings/pandoc.html)

## Repository

Part of the [aozora](https://github.com/P4suta/aozora) workspace.
Dual-licensed Apache-2.0 OR MIT.

[pandoc-ast]: https://hackage.haskell.org/package/pandoc-types
