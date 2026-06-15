# NodeKind::Warichu

Wire tag: `warichu` — split-line annotation (割注). Two text runs
are stacked into a single line of the surrounding text.

## Source examples

```text
［＃割り注］上の段／下の段［＃割り注終わり］
```

## Rendered HTML

```html
<span class="aozora-warichu">
  <span class="aozora-warichu-upper">上の段</span>
  <span class="aozora-warichu-lower">下の段</span>
</span>
```

## Serialize output

Round-trips to the explicit `［＃割り注］...／...［＃割り注終わり］`.

## AST shape

```rust,ignore
pub struct Warichu<'src> {
    pub upper: Content<'src>,
    pub lower: Content<'src>,
}
```

`upper` / `lower` are plain [`Content`](https://docs.rs/aozora/latest/aozora/syntax/borrowed/enum.Content.html);
empty halves are valid (one-sided warichu).

## When emitted

The single-line `［＃割り注］...［＃割り注終わり］` form is
inline-classified; multi-line `［＃割注］` containers become a
[Container](container.md) of kind `Warichu`.

## Diagnostics

None on well-formed input.

## Related kinds

- [Container](container.md) — multi-line counterpart.
