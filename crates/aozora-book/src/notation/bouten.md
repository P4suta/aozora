# Bouten / bousen (傍点・傍線)

**Bouten** (傍点) are emphasis dots placed beside characters in
vertical text — the Japanese typographic equivalent of italic or bold.
**Bousen** (傍線) are the same idea with a line instead of dots. The
spec recognises eleven dot variants and six line variants; aozora
accepts every one.

## Notation forms

Two indirection styles, both common in real corpus:

```text
［＃「平和」に傍点］           ← target-by-quoting
平和［＃「平和」に傍点］        ← redundant explicit copy (also accepted)
［＃ここから傍点］平和［＃ここで傍点終わり］  ← container form
```

The target-by-quoting form is by far the most common: the inline
annotation looks *backwards* in the text for the most recent
occurrence of the quoted string and applies the bouten to that run.

## Variant catalogue

| Slug | Source kanji | Renders as |
|---|---|---|
| `sesame` | 傍点 | small black sesame `﹅` |
| `white_sesame` | 白ゴマ傍点 | small white sesame `﹆` |
| `circle` | 丸傍点 | filled circle `●` |
| `white_circle` | 白丸傍点 | open circle `○` |
| `dot` | 黒点傍点 | bold black dot |
| `triangle` | 三角傍点 | filled triangle |
| `white_triangle` | 白三角傍点 | open triangle |
| `bullseye` | 二重丸傍点 | bullseye |
| `kotenten` | コ点傍点 | small katakana ko-mark |
| `kotenten_white` | 白コ点傍点 | white ko-mark |
| `linear` | 線傍点 | dotted underline |
| `single_line` | 傍線 | single line |
| `double_line` | 二重傍線 | double line |
| `dashed_line` | 鎖線 | dashed line |
| `wavy_line` | 波線 | wavy line |
| `chained_line` | 二重鎖線 | double dashed line |
| `under_dotted` | 下線 | dotted underline |

Each variant has a stable `BoutenKind::slug()` that the HTML renderer
emits as a class name (e.g. `<em class="aozora-bouten-sesame">`). See
[Architecture → HTML renderer](../arch/renderer.md) for the full
class-name scheme.

## Default rendering

aozora emits `<em class="aozora-bouten-<slug>">…</em>` so that an
external stylesheet can pick the visual treatment per variant.
Default CSS hooks live at the consumer side; the parser ships no
stylesheet of its own.

```html
<!-- 平和［＃「平和」に傍点］ -->
平和<em class="aozora-bouten-sesame">平和</em>
```

(The redundant copy is intentional — the `［＃…］` indirection
*re-emits* the target wrapped in `<em>`, leaving the original run
in place. The HTML rendering matches what print Aozora Bunko output
does in practice.)

## Container form

For runs that span multiple lines or include other annotations, use
the container form:

```text
［＃ここから傍点］
平和は手の届かないものだった。
そして、戦争もまた。
［＃ここで傍点終わり］
```

Renders as:

```html
<em class="aozora-bouten-sesame">
平和は手の届かないものだった。
そして、戦争もまた。
</em>
```

The opening directive can be any of the variant openers (`ここから二重傍線`,
`ここから波線`, …); the matching closer must use the same family
(`ここで傍線終わり` for any 線 variant, `ここで傍点終わり` for any 点
variant). Mismatched closers fire diagnostic
[`E0004`](diagnostics.md#E0004).

## AST shape

```rust
pub struct Bouten<'src> {
    pub target: &'src str,        // the run wrapped in emphasis
    pub kind:   BoutenKind,       // one of 17 variants
    pub form:   BoutenForm,       // Indirect | Inline | Container
    pub span:   Span,
}
```

`BoutenKind` is a flat enum with slug accessors; see the
[rustdoc](../ref/api.md) for the exact variant list.

## See also

- [Notation overview](overview.md) — how this fits with the other
  inline annotations.
- [Diagnostics catalogue](diagnostics.md) — `E0004`, `W0003`.
