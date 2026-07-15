# NodeKind::AlignEnd

Inspect tag: `alignEnd` — right-edge alignment marker (字上げ).

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

## Source output

Round-trips to `［＃地付き］` / `［＃地からN字上げ］`.

## When emitted

The classify stage matches the directive form. Paired alignment regions
(`［＃ここから地から N 字上げ］` … `［＃ここで字上げ終わり］`) are
`container` instead.

## Diagnostics

None.

## Related kinds

- `indent` — left-edge counterpart.
- `container` — paired regions
  (`RegionFormat::AlignEnd`).
