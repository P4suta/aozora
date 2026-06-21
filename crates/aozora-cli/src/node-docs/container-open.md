# NodeKind::ContainerOpen

Inspect tag: `containerOpen` — paired-container open boundary marker.

This variant only appears in `NodeRef`-flavoured JSON output (e.g.
`nodes`); the structural [`Node::Container`](container.md)
payload covers the wrapping construct itself.

## Source examples

```text
［＃ここから2字下げ］     <- ContainerOpen
indented body
［＃ここで字下げ終わり］   <- ContainerClose
```

## Rendered HTML

The default HTML renderer routes the open / close pair through
`visit_container_open` / `visit_container_close` and emits the
opening `<div class="aozora-container-...">` wrapping the body.

## Source output

Round-trips together with the matching close to the
`［＃ここから…］...［＃ここで…終わり］` form.

## AST shape

`NodeRef::BlockOpen(ContainerKind)` — see
[ContainerKind](container.md).

## When emitted

The pair stage pairs the open / close brackets; the classify stage's
normalised text emits a `BlockOpen` PUA sentinel at the position of the
opener so the registry can dispatch the open event during walking.

## Diagnostics

`unclosed_bracket` if the open never finds a matching close.

## Related kinds

- [ContainerClose](container-close.md) — paired close-side counterpart.
- [Container](container.md) — the structural payload variant.
