# NodeKind::CombineUpright

Inspect tag: `combineUpright` — horizontal text inside a vertical
writing-mode run (縦中横, "vertical-with-horizontal-inside").

## Source examples

The directive follows its target — it points back at text already
written, so `昭和［＃「12」は縦中横］年` matches nothing.

```text
昭和12年［＃「12」は縦中横］
```

## Rendered HTML

```html
<p>昭和<span class="aozora-combine-upright">12</span>年</p>
```

Downstream CSS gives the span `text-combine-upright: all` for proper
vertical-writing display.

## Source output

Round-trips to `［＃「target」は縦中横］`.

## When emitted

The classify stage matches the directive `［＃「TARGET」は縦中横］` and resolves
TARGET in preceding text, then emits with the matched span.

## Diagnostics

`aozora::lex::unclosed_bracket` if `［＃` is unmatched.

## Related kinds

- `directive` — fallback if target resolution fails.
