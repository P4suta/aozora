# NodeKind::Annotation

Wire tag: `annotation` — generic `［＃...］` annotation that no
specific recogniser claimed.

## Source examples

```text
text［＃任意のメモ］more
text［＃ふりがな付きの説明］more
```

## Rendered HTML

```html
<span class="aozora-annotation" title="..."></span>
```

The default renderer suppresses the body; downstream filters can
match on `aozora-annotation` to surface the comment.

## Serialize output

Round-trips to `［＃<raw>］`.

## AST shape

```rust,ignore
pub struct Annotation<'src> {
    pub raw: NonEmptyStr<'src>,
    pub kind: AnnotationKind,
}
```

`AnnotationKind` discriminates the recognised sub-variants
(`Unknown`, `AsIs`, `TextualNote`, `InvalidRubySpan`, …); `raw`
carries the raw bracket body for any further analysis.

## When emitted

Phase 3 reaches `［＃...］` after no specific recogniser matched.
`Annotation` is the fallback that *always* preserves the user's
content rather than dropping it.

## Diagnostics

None — Annotation *is* the recovery path for unrecognised
directives. A genuine invalid-bracket diagnostic
(`unclosed_bracket` / `unmatched_close`) appears separately.

## Related kinds

- [Bouten](bouten.md) — recognised variant.
- [Kaeriten](kaeriten.md) — recognised variant.
- [Sashie](sashie.md) — recognised variant.
