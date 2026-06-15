# NodeKind::Indent

Wire tag: `indent` — single-line `［＃N字下げ］` indent marker.

## Source examples

```text
［＃2字下げ］
［＃3字下げ］もう一段下げる
```

## Rendered HTML

```html
<span class="aozora-indent" data-amount="2"></span>
```

CSS controls the actual padding (typically `padding-inline-start: Nem`).

## Serialize output

Round-trips to `［＃N字下げ］`.

## AST shape

```rust,ignore
pub struct Indent {
    pub amount: u8,
}
```

## When emitted

Phase 3 matches the digraph plus a numeric prefix and emits a
single inline marker. For *paired* indent regions (`［＃ここから2字下げ］`
… `［＃ここで字下げ終わり］`), see [Container](container.md).

## Diagnostics

None on well-formed input.

## Related kinds

- [Container](container.md) — paired indent / dedent regions
  (`ContainerKind::Indent`).
- [AlignEnd](align-end.md) — right-edge alignment counterpart.
