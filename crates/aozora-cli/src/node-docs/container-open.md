# NodeKind::ContainerOpen

Inspect tag: `containerOpen` — paired-container open boundary marker.

This variant only appears in `NodeRef`-flavoured JSON output (e.g.
`nodes`); the structural `container`
payload covers the wrapping construct itself.

## Source examples

```text
［＃ここから2字下げ］     <- ContainerOpen
indented body
［＃ここで字下げ終わり］   <- ContainerClose
```

## Rendered HTML

The opening `<div class="aozora-container-...">` that wraps the
body, closed by the matching `containerClose`.

## Source output

Round-trips together with the matching close to the
`［＃ここから…］...［＃ここで…終わり］` form.

## When emitted

The pair stage pairs the open / close brackets; the classify stage's
normalised text emits a `BlockOpen` PUA sentinel at the position of the
opener so the registry can dispatch the open event during walking.

## Diagnostics

`unclosed_bracket` if the open never finds a matching close.

## Related kinds

- `containerClose` — paired close-side counterpart.
- `container` — the structural payload variant.
