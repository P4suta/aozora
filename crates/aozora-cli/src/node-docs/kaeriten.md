# NodeKind::Kaeriten

Inspect tag: `kaeriten` — kanbun reading-order marker (返り点).

## Source examples

The mark stands alone inside the brackets. There is no `返り点` keyword.

```text
読［＃一］本
春眠［＃レ］暁
```

## Rendered HTML

```html
<p>読<sup class="aozora-kaeriten">一</sup>本</p>
```

The mark is the element's text. CSS positions the `sup` for the
writing mode.

## Source output

Round-trips to `［＃mark］`.

## When emitted

The classify stage recognises a bracketed body that is exactly a
reading-order mark.

Marks come in ordered families — `一` < `二` < `三` < `四`, `上` < `中`
< `下`, `甲` < `乙` < `丙` < `丁` — and a mark above the base rank needs
a same-family base somewhere in the document. `レ` and the 送り仮名
`（X）` form are standalone and never ladder.

## Diagnostics

- `aozora::lex::bracketed_kaeriten_no_pair` — a mark whose family base
  is missing, e.g. `［＃二］` with no `［＃一］`. Severity `error`: this
  is the one kind whose classification can hard-fail.

## Related kinds

None.
