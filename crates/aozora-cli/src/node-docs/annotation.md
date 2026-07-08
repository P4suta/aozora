# NodeKind::Directive

Inspect tag: `annotation` — generic `［＃...］` annotation that no
specific recogniser claimed.

## Source examples

```text
text［＃任意のメモ］more
text［＃ふりがな付きの説明］more
```

## Rendered HTML

```html
<span class="aozora-directive" title="..."></span>
```

The default renderer suppresses the body; downstream filters can
match on `aozora-directive` to surface the comment.

## Source output

Round-trips to `［＃<raw>］`.

## AST shape

```rust,ignore
pub struct Directive<'src> {
    pub raw: NonEmptyStr<'src>,
    pub kind: DirectiveKind,
}
```

`DirectiveKind` discriminates the recognised sub-variants
(`Unknown`, `Sic`, `BaseTextVariant`, …); `raw`
carries the raw bracket body for any further analysis.

## When emitted

The classify stage reaches `［＃...］` after no specific recogniser matched.
`Directive` is the fallback that *always* preserves the user's
content rather than dropping it.

## Diagnostics

None — Directive *is* the recovery path for unrecognised
directives. A genuine invalid-bracket diagnostic
(`unclosed_bracket` / `unmatched_close`) appears separately.

## Related kinds

- [Bouten](bouten.md) — recognised variant.
- [Kaeriten](kaeriten.md) — recognised variant.
- [Illustration](sashie.md) — recognised variant.
