# NodeKind::PageBreak

Wire tag: `pageBreak` — `［＃改ページ］` page break marker.

## Source examples

```text
end of chapter
［＃改ページ］
beginning of next chapter
```

## Rendered HTML

```html
<div class="aozora-page-break"></div>
```

CSS gives the div a `page-break-before: always` for paged media
(EPUB / print).

## Serialize output

Round-trips to `［＃改ページ］\n`.

## AST shape

`AozoraNode::PageBreak` is a unit variant — no payload.

## When emitted

Phase 3 sees `［＃改ページ］` and emits a single `BlockLeaf`
classification covering the whole bracket span.

## Diagnostics

None on well-formed input.

## Related kinds

- [SectionBreak](section-break.md) — `［＃改丁］` family.
