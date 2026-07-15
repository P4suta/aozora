# NodeKind::Gaiji

Inspect tag: `gaiji` — out-of-character-set glyph reference. The
historical Aozora-Bunko notation for characters Shift_JIS could
not encode; modern files mostly use them for genuine non-Unicode
glyphs.

## Source examples

```text
※［＃「木＋世」、第3水準1-85-54］
```

The `※` (`U+203B`) flags the construct; `［＃description、mencode］`
carries a human description and a structured JIS / Unicode
identifier. The mencode is what resolves — the description is a
label for a reader.

## Rendered HTML

Resolved, which is the common case:

```html
<p><span class="aozora-gaiji" data-codepoint="U+6798">枘</span></p>
```

Unresolved — no mencode, or one no table knows:

```html
<p><span class="aozora-gaiji" data-description="変な字">変な字</span></p>
```

Both carry the glyph or the description as the element's text, so a
reader with no stylesheet still sees something. `data-codepoint` lists
one `U+XXXX` per scalar, space-separated, because a resolved gaiji may
be a combining sequence.

## Source output

Round-trips to `※［＃description、mencode］`.

## When emitted

The classify stage sees the `※［＃…］` digraph and parses the
description / mencode payload. The encoding crate's resolver lifts the
mencode into a Unicode character when a table has one.

## Diagnostics

None on a well-formed `※［＃…］`. An unresolvable reference is not an
error — it renders as its description.

## Related kinds

- `directive` — fallback when the body is malformed.
