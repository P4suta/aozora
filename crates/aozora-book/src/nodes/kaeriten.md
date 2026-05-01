# NodeKind::Kaeriten

Wire tag: `kaeriten` — kanbun reading-order marker (返り点).

## Source examples

```text
読［＃返り点 一・二］本
```

## Rendered HTML

```html
<sup class="aozora-kaeriten" data-mark="一・二"></sup>
```

CSS positions the sup glyph appropriately for vertical / horizontal
writing mode.

## Serialize output

Round-trips to `［＃返り点 mark］`.

## AST shape

```rust,ignore
pub struct Kaeriten<'src> {
    pub mark: NonEmptyStr<'src>,
}
```

## When emitted

Phase 3 matches `返り点` keyword + marker payload. Empty marker
rejected upstream.

## Diagnostics

None on well-formed input.

## Related kinds

None.
