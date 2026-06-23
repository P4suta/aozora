# NodeKind::Ruby

Inspect tag: `ruby` — base text + reading annotation. The most common
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

## Source output

`to_source()` emits the **canonical bare** form `base《reading》`,
adding an explicit `｜` only when a bare reading would re-parse to a
different base (see ADR 0002/0003). The parse → to_source → parse
round-trip is a fixed point regardless of which form the source used;
`to_source_verbatim()` instead replays the author's exact bytes.

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

The classify stage classifies a `《…》` pair as ruby when the preceding run is a
sequence of CJK / kana / latin glyphs and the close is followed by
neither a glyph (which would extend the base further) nor a stray
opener.

## Diagnostics

- `aozora::lex::unclosed_bracket` — unbalanced `《` reaches EOF.
- `aozora::lex::unmatched_close` — stray `》` with no matching open.

## Related kinds

- [AngleQuote](angle-quote.md) — `≪…≫` double-angle quotation
  (displays as `《…》`).
- [Directive::InvalidRubySpan](annotation.md) — fallback when the
  ruby pair could not be parsed cleanly.
