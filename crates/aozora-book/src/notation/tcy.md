# 縦中横 (tate-chū-yoko)

**縦中横** (tate-chū-yoko, "horizontal in vertical") is a typographic
construct that lays a short run — usually digits, Latin letters, or
mixed punctuation — *horizontally* inside otherwise vertical text. In
print, it is the common treatment for two- or three-digit numbers in
a vertical paragraph.

## Notation

The annotation always uses the indirect-quoting form:

```text
昭和27［＃「27」は縦中横］年生まれ
```

Renders as:

```html
昭和<span class="aozora-combine-upright">27</span>年生まれ
```

The `［＃…］` directive looks back through the most recent text and
applies the tcy treatment to the most recent occurrence of the
quoted run. When the target sits **immediately before** the bracket
(as above) the lowering pass folds it into the node, so the
combine-upright `<span>` is the sole copy — the same forward-reference
mechanism bouten uses. When the target is recognised earlier but **not**
adjacent (e.g. `昭和27年生まれ［＃「27」は縦中横］`, where 年生まれ
intervenes), it stays `ForwardOrigin::Referenced`: the literal is left in
its run and the streaming renderer drops the markup rather than
duplicating the text — see [傍点](bouten.md#default-rendering).

## No container form

Unlike most block decorations, 縦中横 has **no**
`［＃ここから縦中横］…［＃ここで縦中横終わり］` container form. The official
notation (spec §6.3) defines only the inline forward reference
`X［＃「X」は縦中横］`; there is no `aozora-combine-upright-block` class and the
recogniser has no `ここから縦中横` opener. A stray block-region marker is
therefore **preserved verbatim** as a hidden `aozora-directive` (it never opens
a block), so no content is lost — it simply is not styled.

For a longer horizontal-in-vertical run (multi-line table data, a Latin phrase
spanning a paragraph), use 横組み (horizontal writing) rather than repeating
縦中横; 縦中横 is for uprighting a short digit/letter cluster inside one column.

## Common targets

| Source | Output |
|---|---|
| `27［＃「27」は縦中横］` | `<span class="aozora-combine-upright">27</span>` |
| `100［＃「100」は縦中横］％` | `<span class="aozora-combine-upright">100</span>％` |
| `A4［＃「A4」は縦中横］` | `<span class="aozora-combine-upright">A4</span>` |
| `&［＃「&」は縦中横］` | `<span class="aozora-combine-upright">&amp;</span>` |

(HTML escapes are handled by the renderer, not the AST.)

## Anchor lookup

The lookup that finds the target run:

1. Scans backwards from the `［＃…］` directive through the current
   line.
2. Stops at the first match for the quoted run.
3. Falls through to the previous line if no match (with an upper
   bound of 64 KiB or one paragraph break, whichever comes first).

If no match is found, diagnostic
[`aozora::lex::tcy_target_not_found`](diagnostics.md#tcy-target-not-found)
fires and the directive degrades to a plain `Directive{Unknown}`.
Authors get the same look-back semantics they'd get from bouten — see
[Bouten](bouten.md) for the symmetric case.

## Why a span, not a flow rotation?

Web renderers reach for `writing-mode: horizontal-tb` inside a
`writing-mode: vertical-rl` parent, but that has poor browser support
and breaks line-break propagation. aozora's HTML output uses a
single class hook (`<span class="aozora-combine-upright">`) so the consuming
stylesheet can decide:

- print stylesheet → `font-feature-settings: "vert"; text-combine-upright: all;`
- screen stylesheet → leave horizontal, set monospace
- e-book renderer → use the renderer's native tcy primitive

Pushing this decision into the HTML output (e.g. emitting an inline
SVG with rotated glyphs) would lock consumers into a specific
typographic model. The class-hook output keeps the HTML semantic and
defers presentation to the consumer.

The renderer still ships no CSS, but the crate now ships a **canonical
reference stylesheet** (`crates/aozora-render/assets/aozora-notation.css`)
whose screen default is exactly `text-combine-upright: all` on
`.aozora-combine-upright`. The playground and VS Code preview adopt it,
so 縦中横 combines correctly in vertical mode out of the box; a consumer
wanting the print or e-book variant above overrides that one rule. See
[ADR-0024](https://github.com/P4suta/aozora/blob/main/docs/adr/0024-canonical-reference-stylesheet.md).

## AST shape

```rust,ignore
pub struct Tcy<'src> {
    pub text: &'src str,
    pub form: TcyForm,    // Inline | Container
    pub span: Span,
}
```

## See also

- [Indent containers](indent.md) — tcy commonly appears inside
  字下げ blocks; the parser applies tcy *after* the indent fence is
  established so the look-back search is bounded by the inner block.
