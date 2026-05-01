# Pandoc integration

The `aozora-pandoc` crate (workspace-internal, available via the
`aozora` CLI) projects a parsed Aozora document into the
[Pandoc AST][pandoc-ast]. Once you have Pandoc JSON, every Pandoc
output format (HTML, EPUB, LaTeX/PDF, DOCX, ODT, MediaWiki, …) is one
shell pipe away.

This is the recommended path if you want to convert Aozora Bunko
notation into anything *other* than the built-in HTML renderer.
Adding a new output format means adding a Pandoc filter (or none, if
the default Span/Div mapping is enough), not extending the parser
crate.

## Quickstart

```sh
# Pandoc JSON to stdout
aozora pandoc input.txt > out.json

# Or pipe through pandoc directly
aozora pandoc input.txt | pandoc -f json -t html
aozora pandoc input.txt | pandoc -f json -t epub3 -o out.epub

# `--format` is shorthand for the pipe (requires pandoc on PATH)
aozora pandoc input.txt --format html > out.html
aozora pandoc -E sjis legacy.txt -t epub > out.epub
```

## Projection rules

Each [`AozoraNode`][nodekind] variant lifts to a Pandoc construct
carrying a stable CSS class so downstream filters or stylesheets can
specialise the rendering:

| Aozora variant         | Pandoc construct          | Class on the construct        |
| ---------------------- | ------------------------- | ----------------------------- |
| `Ruby`                 | `Span`                    | `aozora-ruby`                 |
|   ↳ base text          | nested `Span`             | `aozora-ruby-base`            |
|   ↳ reading text       | nested `Span`             | `aozora-ruby-reading`         |
| `Bouten`               | `Span` over target text   | `aozora-bouten`               |
| `TateChuYoko`          | `Span`                    | `aozora-tate-chu-yoko`        |
| `Gaiji`                | `Span` carrying mencode   | `aozora-gaiji`                |
| `Indent`, `AlignEnd`   | empty `Span` (marker)     | `aozora-indent` / `align-end` |
| `Warichu`              | `Span` with two children  | `aozora-warichu`              |
| `DoubleRuby`           | `Span`                    | `aozora-double-ruby`          |
| `Annotation`, `Kaeriten`, `HeadingHint` | empty `Span` carrying raw | `aozora-annotation` / etc.    |
| `PageBreak`            | `HorizontalRule` block    | (n/a — semantic block)        |
| `SectionBreak`         | empty `Div`               | `aozora-section-break`        |
| `AozoraHeading`        | `Header` block            | `aozora-heading`              |
| `Sashie`               | `Para` with `Image`       | `aozora-sashie`               |
| Container (字下げ等)   | `Div` wrapping inner blocks | `aozora-container-indent` / etc. |

The structural attribute `kvs` (Pandoc's third Attr tuple) carries
non-textual metadata (bouten kind / position, gaiji description /
mencode, indent amount, container kind). Filters that want
format-native rendering pattern-match on the class + kvs.

## Why a Pandoc projection at all

Aozora notation has rich semantic markup (ruby, bouten, tate-chu-yoko,
gaiji…) that no single Pandoc native construct captures. The naive
shortcut of emitting `RawInline("html", "<ruby>…</ruby>")` would only
work for the HTML writer; every other Pandoc output format would
strip the raw HTML and lose the meaning.

By lifting each Aozora variant to a `Span` / `Div` with a stable
class, the same JSON renders sensibly across every Pandoc format
*today* (each format's writer renders Span as a stylable container)
and stays open for richer format-native rendering *tomorrow* via
filters. That's the same pattern Pandoc itself uses for
`[content]{.smallcaps}` — semantic in the AST, format-specific in the
writer.

## Architecture

The library entry point is [`aozora_pandoc::to_pandoc`][lib]:

```rust,ignore
use aozora::Document;
use aozora_pandoc::to_pandoc;

let doc = Document::new(std::fs::read_to_string("input.txt")?);
let pandoc = to_pandoc(&doc.parse());
let json = serde_json::to_string(&pandoc)?;
```

`aozora-cli` wires that into `aozora pandoc` so binary consumers
don't need to write Rust.

[pandoc-ast]: https://hackage.haskell.org/package/pandoc-types
[nodekind]: ../wire/overview.md
[lib]: https://docs.rs/aozora-pandoc
