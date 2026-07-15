# aozora-pandoc

Projects a parsed aozora document into the
[Pandoc AST](https://hackage.haskell.org/package/pandoc-types). Once you
have Pandoc JSON, every Pandoc output format — EPUB, LaTeX, DOCX, ODT,
MediaWiki — is one pipe away. This is the route to anything other than
the built-in HTML renderer.

Each node lifts to a `Span` or `Div` carrying a stable CSS class
(`aozora-ruby`, `aozora-bouten`, …) and its data as attributes, so
filters and stylesheets can specialise the rendering.

## Use

The supported surface is the CLI:

```sh
aozora pandoc input.txt | pandoc -f json -t epub3 -o out.epub
aozora pandoc input.txt --format html > out.html   # shorthand for the pipe
```

> **Experimental crate.** The Rust `to_pandoc` API can change in any
> release. See [docs.rs/aozora-pandoc](https://docs.rs/aozora-pandoc).

Part of the [aozora](https://github.com/P4suta/aozora) workspace.
Dual-licensed Apache-2.0 OR MIT.
