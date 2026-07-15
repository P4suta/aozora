# NodeKind::SectionBreak

Inspect tag: `sectionBreak` — section breaks (改丁 / 改段 / 改見開き).

## Source examples

```text
［＃改丁］
［＃改段］
［＃改見開き］
```

## Rendered HTML

```html
<div class="aozora-section-break aozora-section-break-kaicho"></div>
```

The second class slot carries the variant slug (`kaicho`, `kaidan`,
`kaimihiraki`, `other`).

## Source output

Round-trips to `［＃改丁］` etc.

## When emitted

The classify stage matches each directive; the kind enum captures which.

## Diagnostics

None on well-formed input.

## Related kinds

- `pageBreak` — finer-grained `［＃改ページ］` variant.
