# NodeKind::Keigakomi

Wire tag: `keigakomi` — ruled-box annotation (罫囲み).

## Source examples

```text
［＃罫囲み］本文［＃罫囲み終わり］
```

## Rendered HTML

```html
<span class="aozora-keigakomi"></span>
```

(Inline marker; the multi-line container form yields a
`<div class="aozora-container-keigakomi">` wrapper instead — see
[Container](container.md).)

## Serialize output

Round-trips to `［＃罫囲み］...［＃罫囲み終わり］`.

## AST shape

```rust,ignore
pub struct Keigakomi;
```

Marker struct with no payload — the surrounding text carries the
content.

## When emitted

Phase 3 sees the inline form. Multi-line keigakomi blocks classify
as [Container](container.md) `Keigakomi`.

## Diagnostics

None on well-formed input.

## Related kinds

- [Container](container.md) — multi-line counterpart.
