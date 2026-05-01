# NodeKind::DoubleRuby

Wire tag: `doubleRuby` — double-bracket bouten (`《《重要》》`).

## Source examples

```text
《《重要》》
```

## Rendered HTML

```html
<em class="aozora-double-ruby">重要</em>
```

CSS typically sets `font-weight: bold` or attaches sidelines for
this construct; the default class hand-off lets stylesheets pick
the visual.

## Serialize output

Round-trips to `《《content》》`.

## AST shape

```rust,ignore
pub struct DoubleRuby<'src> {
    pub content: NonEmpty<Content<'src>>,
}
```

`content` is `NonEmpty` — empty `《《》》` is rejected upstream and
falls through to plain text rather than producing an empty node.

## When emitted

Phase 3 sees `《《` as a single tokenised opener (not two `《`); the
classifier matches `《《...》》` as a single pair and emits the
node.

## Diagnostics

`unclosed_bracket` for `《《` without `》》`.

## Related kinds

- [Ruby](ruby.md) — single-bracket variant.
