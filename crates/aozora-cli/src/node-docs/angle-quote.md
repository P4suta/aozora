# NodeKind::AngleQuote

Inspect tag: `angleQuote` — double-angle quotation (二重山括弧).

A 底本's twin angle brackets `《…》` would collide with the ruby
markers `《…》` (U+300A/U+300B), so Aozora Bunko **input** encodes them
as `≪…≫` (U+226A/U+226B). The renderer restores the **display** form
`《…》`.

## Source examples

```text
≪重要≫
```

底本 `《重要》` → aozora text `≪重要≫` → display `《重要》`.

## Rendered HTML

```html
<span class="aozora-angle-quote">《重要》</span>
```

The `《…》` display glyphs (U+300A/U+300B) are restored inside the span;
stylesheets target `.aozora-angle-quote` for any further treatment.

## Serialize output

Round-trips to the input form `≪content≫` (U+226A/U+226B).

## AST shape

```rust,ignore
pub struct AngleQuote<'src> {
    pub content: NonEmpty<Content<'src>>,
}
```

`content` is `NonEmpty` — empty `≪≫` is rejected upstream and falls
through to plain text rather than producing an empty node.

## When emitted

Phase 1 tokenises `≪` / `≫` (U+226A/U+226B) as ordinary single-character
triggers; Phase 3 pairs `≪…≫` into one `AngleQuote` node. A stray
底本-style `《《…》》` is **not** this node — it is two ruby openers and
yields a `nested-ruby` diagnostic with plain fallback.

## Diagnostics

- `aozora::lex::unclosed_bracket` — `≪` reaches EOF without `≫`.
- `aozora::lex::unmatched_close` — stray `≫` with no matching open.

## Related kinds

- [Ruby](ruby.md) — `《…》` reading marker (the colliding notation).
