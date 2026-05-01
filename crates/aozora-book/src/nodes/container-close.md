# NodeKind::ContainerClose

Wire tag: `containerClose` — paired-container close boundary marker.

`NodeRef`-only counterpart of [ContainerOpen](container-open.md).

## Source examples

```text
［＃ここから2字下げ］     <- ContainerOpen
body
［＃ここで字下げ終わり］   <- ContainerClose
```

## Rendered HTML

Routed through `visit_container_close`; the default renderer emits
the closing `</div>` of the
`<div class="aozora-container-...">` opened by the matching
`ContainerOpen`.

## Serialize output

Round-trips with the matching open.

## AST shape

`NodeRef::BlockClose(ContainerKind)`.

## When emitted

Phase 3 normalised-text emits a `BlockClose` PUA sentinel at the
matching close position.

## Diagnostics

`unmatched_close` if the close has no open partner — in which case
no `ContainerClose` is emitted and the close-bracket bytes flow
through as plain.

## Related kinds

- [ContainerOpen](container-open.md) — open-side counterpart.
- [Container](container.md) — structural payload.
