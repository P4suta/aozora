# NodeKind::AlignEnd

Wire tag: `alignEnd` — right-edge alignment marker (字上げ).

## Source examples

```text
［＃地付き］
［＃地から3字上げ］
```

## Rendered HTML

```html
<span class="aozora-align-end" data-offset="0"></span>
```

`offset` is `0` for 地付き, `N` for 地から N 字上げ.

## Serialize output

Round-trips to `［＃地付き］` / `［＃地からN字上げ］`.

## AST shape

```rust,ignore
pub struct AlignEnd {
    pub offset: u8,
}
```

## When emitted

Phase 3 matches the directive form. Paired alignment regions
(`［＃ここから地から N 字上げ］` … `［＃ここで字上げ終わり］`) are
[Container](container.md) instead.

## Diagnostics

None.

## Related kinds

- [Indent](indent.md) — left-edge counterpart.
- [Container](container.md) — paired regions
  (`ContainerKind::AlignEnd`).
