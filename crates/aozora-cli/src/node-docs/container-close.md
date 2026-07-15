# NodeKind::ContainerClose

Inspect tag: `containerClose` — paired-container close boundary marker.

`NodeRef`-only counterpart of `containerOpen`.

## Source examples

```text
［＃ここから2字下げ］     <- ContainerOpen
body
［＃ここで字下げ終わり］   <- ContainerClose
```

## Rendered HTML

The closing `</div>` of the `<div class="aozora-container-...">`
that the matching `containerOpen` opened.

## Source output

Round-trips with the matching open.

## When emitted

The classify stage's normalised text emits a `BlockClose` PUA sentinel
at the matching close position.

## Diagnostics

`unmatched_close` if the close has no open partner — in which case
no `ContainerClose` is emitted and the close-bracket bytes flow
through as plain.

## Related kinds

- `containerOpen` — open-side counterpart.
- `container` — structural payload.
