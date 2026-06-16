# NodeKind::SectionBreak

Wire tag: `sectionBreak` — section breaks (改丁 / 改段 / 改見開き).

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

## Serialize output

Round-trips to `［＃改丁］` etc.

## AST shape

```rust,ignore
AozoraNode::SectionBreak(SectionKind)
```

`SectionKind` is `Choho` (改丁) / `Dan` (改段) / `Spread` (改見開き).

## When emitted

Phase 3 matches each directive; the kind enum captures which.

## Diagnostics

None on well-formed input.

## Related kinds

- [PageBreak](page-break.md) — finer-grained `［＃改ページ］` variant.
