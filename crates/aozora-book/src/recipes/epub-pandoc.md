# EPUB via Pandoc

**Problem.** You want an EPUB (or LaTeX/PDF, DOCX, ODT, …) out of an
Aozora Bunko source — anything the built-in HTML renderer does not
produce.

## Solution

The `aozora` binary projects a parsed document into the
[Pandoc AST](../bindings/pandoc.md) as JSON. Pipe that JSON into
`pandoc`, and every Pandoc output format is one writer away. For EPUB:

```sh
aozora pandoc input.txt | pandoc -f json -t epub3 -o out.epub
```

That is the whole recipe. The same pipe reaches the other formats by
swapping the `-t` writer:

```sh
aozora pandoc input.txt | pandoc -f json -t latex  -o out.tex
aozora pandoc input.txt | pandoc -f json -t docx   -o out.docx
aozora pandoc input.txt | pandoc -f json -t html   > out.html
```

Shift_JIS source decodes with `-E sjis`, exactly as for `render` /
`check`:

```sh
aozora pandoc -E sjis crime.txt | pandoc -f json -t epub3 -o crime.epub
```

`aozora pandoc` also has a `--format` / `-t` shorthand that runs the
pipe for you when `pandoc` is on `PATH`:

```sh
aozora pandoc input.txt -t epub > out.epub
```

## Expected output

`out.epub` — a valid EPUB 3 container. Each Aozora construct lifts to
a Pandoc `Span` / `Div` carrying a stable CSS class (`aozora-ruby`,
`aozora-bouten`, …), so you can style or filter them per format. The
[projection-rules table](../bindings/pandoc.md#projection-rules) lists
every variant's mapping.

## Why a Pandoc projection at all

Aozora notation has rich semantic markup (ruby, bouten, 縦中横,
gaiji) that no single Pandoc native construct captures. Emitting raw
HTML would only survive the HTML writer; every other format would
strip it. Lifting each variant to a classed `Span`/`Div` instead means
the same JSON renders sensibly across *every* Pandoc format today and
stays open to richer format-native rendering via filters tomorrow.
Adding a new output format is a Pandoc filter, never a parser change.

## See also

- [Pandoc AST projection](../bindings/pandoc.md) — the full projection
  table, the `kvs` metadata contract, and the library entry point.
- [Choosing a binding → By output format](../bindings/choosing.md#by-output-format).
- [CLI reference](../ref/cli.md) — `aozora pandoc` flags.
