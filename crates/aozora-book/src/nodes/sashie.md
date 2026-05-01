# NodeKind::Sashie

Wire tag: `sashie` — illustration reference (挿絵).

## Source examples

```text
［＃挿絵（cover.png）入る］
［＃挿絵（pages/03.jpg、第3章扉絵）入る］
```

## Rendered HTML

```html
<figure class="aozora-sashie">
  <img src="cover.png" alt="">
</figure>
```

When a caption is present it lands as a `<figcaption>` next to the
`<img>`.

## Serialize output

Round-trips to `［＃挿絵（path[、caption]）入る］`.

## AST shape

```rust,ignore
pub struct Sashie<'src> {
    pub file: NonEmptyStr<'src>,
    pub caption: Option<Content<'src>>,
}
```

Empty `file` is rejected upstream — the construct cannot ship a
nameless image.

## When emitted

Phase 3 matches the `挿絵（…）入る` digraph and parses out the path
+ optional caption.

## Diagnostics

None on well-formed input.

## Related kinds

- [Annotation](annotation.md) — fallback when the directive is
  malformed.
