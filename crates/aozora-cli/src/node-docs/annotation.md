# NodeKind::Directive

Inspect tag: `directive` — generic `［＃...］` annotation that no
specific recogniser claimed.

## Source examples

```text
text［＃任意のメモ］more
text［＃ふりがな付きの説明］more
```

## Rendered HTML

```html
<p>text<span class="aozora-directive" hidden>［＃任意のメモ］</span>more</p>
```

The raw text stays in the DOM and `hidden` keeps it off the page, so a
filter can match `aozora-directive` and surface it.

## Source output

Round-trips to `［＃<raw>］`.

## When emitted

The classify stage reaches `［＃...］` after no specific recogniser matched.
`Directive` is the fallback that *always* preserves the user's
content rather than dropping it.

## Diagnostics

None — Directive *is* the recovery path for unrecognised
directives. A genuine invalid-bracket diagnostic
(`unclosed_bracket` / `unmatched_close`) appears separately.

## Related kinds

- `bouten` — recognised variant.
- `kaeriten` — recognised variant.
- `illustration` — recognised variant.
