# NodeKind::Indent

Inspect tag: `indent` — single-line `［＃N字下げ］` indent marker.

## Source examples

```text
［＃2字下げ］
［＃3字下げ］もう一段下げる
```

## Rendered HTML

```html
<p><span class="aozora-indent aozora-indent-2" data-amount="2"></span>本文</p>
```

The amount appears twice: as a modifier class for CSS and as
`data-amount` for anything reading the DOM.

CSS controls the actual padding (typically `padding-inline-start: Nem`).

## Source output

Round-trips to `［＃N字下げ］`.

## When emitted

The classify stage matches the digraph plus a numeric prefix and emits a
single inline marker. For *paired* indent regions (`［＃ここから2字下げ］`
… `［＃ここで字下げ終わり］`), see `container`.

## Diagnostics

None on well-formed input.

## Related kinds

- `container` — paired indent / dedent regions
  (`RegionFormat::Indent`).
- `alignEnd` — right-edge alignment counterpart.
