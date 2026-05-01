# NodeKind::AozoraHeading

Wire tag: `heading` — Aozora 見出し (window / sub heading).

## Source examples

```text
［＃見出し］序章［＃見出し終わり］
```

## Rendered HTML

```html
<h2 class="aozora-heading aozora-heading-window">序章</h2>
```

The Pandoc projection uses level 2 for `Window`, level 3 for `Sub`.

## Serialize output

Round-trips to `［＃<kind>見出し］...［＃<kind>見出し終わり］`.

## AST shape

```rust,ignore
pub struct AozoraHeading<'src> {
    pub kind: AozoraHeadingKind,
    pub text: NonEmpty<Content<'src>>,
}
```

`AozoraHeadingKind` is `Window` (窓見出し) or `Sub` (副見出し).

## When emitted

Phase 3 matches the keyword `見出し` family and binds the body run.

## Diagnostics

None on well-formed input.

## Related kinds

- [HeadingHint](heading-hint.md) — forward-reference style heading
  hint.
