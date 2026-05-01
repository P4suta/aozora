# NodeKind::Gaiji

Wire tag: `gaiji` — out-of-character-set glyph reference. The
historical Aozora-Bunko notation for characters Shift_JIS could
not encode; modern files mostly use them for genuine non-Unicode
glyphs.

## Source examples

```text
※［＃「木＋吶のつくり」、第3水準1-85-54］
```

The `※` (`U+203B`) flags the construct; `［＃description、mencode］`
carries the human description and a structured Mojikyō / JIS / U+
identifier.

## Rendered HTML

```html
<span class="aozora-gaiji" title="木＋吶のつくり" data-mencode="第3水準1-85-54">〓</span>
```

The fallback glyph `〓` (U+3013, "geta mark") is the conventional
Japanese typesetting placeholder for missing glyphs. When the
resolver finds a Unicode mapping the inner text becomes the
resolved character instead of the geta mark.

## Serialize output

Round-trips to `※［＃description、mencode］`.

## AST shape

```rust,ignore
pub struct Gaiji<'src> {
    pub description: &'src str,
    pub ucs: Option<Resolved>,
    pub mencode: Option<&'src str>,
}
```

`Resolved` is either a single Unicode scalar or one of 25
predefined static combining sequences (e.g. か゚ — `か` + the IPA
voicing-pair-mark — kept as a static constant so the borrowed-AST
stays `Copy`).

## When emitted

Phase 3 sees the `※[#…]` digraph and parses the description /
mencode payload. The encoding crate's gaiji resolver lifts the
mencode reference into a Unicode character when one exists.

## Diagnostics

None on a well-formed `※[#...]`. Ambiguous descriptions land as
`Annotation::Unknown` instead of `Gaiji`.

## Related kinds

- [Annotation](annotation.md) — fallback when description is
  malformed.
