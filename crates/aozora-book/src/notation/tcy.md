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

## Container form

For longer mixed-orientation runs (multi-line table data, Latin
abbreviations spanning a paragraph), the container form sits inside
an outer indent block:

```text
［＃ここから縦中横］
27 / 100 = 0.27
［＃ここで縦中横終わり］
```

Renders as:

```html
<div class="aozora-combine-upright-block">
27 / 100 = 0.27
</div>
```

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
