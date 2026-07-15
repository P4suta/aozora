# NodeKind::Warichu

Inspect tag: `warichu` — split-line annotation (割注). Two text runs
are stacked into a single line of the surrounding text.

## Source examples

```text
［＃割り注］上の段／下の段［＃割り注終わり］
```

## Rendered HTML

```html
<p><span class="aozora-warichu">上の段／下の段</span></p>
```

## Source output

Round-trips to the explicit `［＃割り注］...／...［＃割り注終わり］`.

## When emitted

The single-line `［＃割り注］...［＃割り注終わり］` form is
inline-classified, with the `／` kept as written; multi-line `［＃割注］`
containers become a `container` instead.

## Diagnostics

None on well-formed input.

## Related kinds

- `container` — multi-line counterpart.
