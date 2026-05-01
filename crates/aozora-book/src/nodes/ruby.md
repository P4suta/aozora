# NodeKind::Ruby

Wire tag: `ruby` — base text + reading annotation. The most common
non-trivial variant in Aozora Bunko.

## Source examples

```text
｜青梅《おうめ》
青梅《おうめ》
```

Both forms classify as `Ruby`; the leading `｜` (`U+FF5C`) makes the
delimiter explicit and lets the parser disambiguate the base run
when ambiguous neighbours could otherwise extend the base.

## Rendered HTML

```html
<ruby>青梅<rp>(</rp><rt>おうめ</rt><rp>)</rp></ruby>
```

`<rp>` parens are emitted so HTML clients without ruby support
still display a readable fallback.

## Serialize output

`serialize()` always emits the explicit-delimiter form
(`｜base《reading》`), so a parse → serialize → parse round-trip is
a fixed point regardless of which form the source used.

## AST shape

```rust,ignore
pub struct Ruby<'src> {
    pub base: NonEmpty<Content<'src>>,
    pub reading: NonEmpty<Content<'src>>,
    pub delim_explicit: bool,
}
```

Both fields are [`NonEmpty<Content>`](../arch/arena.md#non-empty-content);
empty base or reading is rejected upstream and never produces a
`Ruby` node.

## When emitted

Phase 3 classifies a `《…》` pair as ruby when the preceding run is a
sequence of CJK / kana / latin glyphs and the close is followed by
neither a glyph (which would extend the base further) nor a stray
opener.

## Diagnostics

- `aozora::lex::unclosed_bracket` — unbalanced `《` reaches EOF.
- `aozora::lex::unmatched_close` — stray `》` with no matching open.

## Related kinds

- [DoubleRuby](double-ruby.md) — `《《…》》` double-bracket variant.
- [Annotation::InvalidRubySpan](annotation.md) — fallback when the
  ruby pair could not be parsed cleanly.
