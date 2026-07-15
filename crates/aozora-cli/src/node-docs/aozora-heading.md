# NodeKind::Heading

Inspect tag: `heading` — Aozora 見出し.

## Source examples

The keyword names the level. Bare `見出し` is not one of them.

```text
［＃中見出し］序章［＃中見出し終わり］
```

## Rendered HTML

```html
<h2 class="aozora-heading aozora-heading-medium">序章</h2>
```

大見出し / 中見出し / 小見出し render as `h1` / `h2` / `h3`. The
window / sub distinction is a separate, orthogonal axis.

## Source output

Round-trips to `［＃<kind>見出し］...［＃<kind>見出し終わり］`.

## When emitted

The classify stage matches the keyword `見出し` family and binds the body run.

## Diagnostics

None on well-formed input.

## Related kinds

- `headingHint` — forward-reference style heading
  hint.
