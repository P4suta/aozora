# NodeKind::TateChuYoko

Wire tag: `tateChuYoko` — horizontal text inside a vertical
writing-mode run (縦中横, "vertical-with-horizontal-inside").

## Source examples

```text
昭和［＃「12」は縦中横］年
```

## Rendered HTML

```html
<span class="aozora-tcy">12</span>
```

Downstream CSS gives the span `text-combine-upright: all` for proper
vertical-writing display.

## Serialize output

Round-trips to `［＃「target」は縦中横］`.

## AST shape

```rust,ignore
pub struct TateChuYoko<'src> {
    pub text: NonEmpty<Content<'src>>,
}
```

## When emitted

Phase 3 matches the directive `［＃「TARGET」は縦中横］` and resolves
TARGET in preceding text, then emits with the matched span.

## Diagnostics

`aozora::lex::unclosed_bracket` if `［＃` is unmatched.

## Related kinds

- [Annotation](annotation.md) — fallback if target resolution fails.
