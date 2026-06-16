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
［＃傍点］平和［＃傍点終わり］     ← range form (bare opener / closer)
```

The target-by-quoting form is by far the most common: the inline
annotation looks *backwards* in the text for the most recent
occurrence of the quoted string and applies the bouten to that run.

## Variant catalogue

aozora recognises eleven variants — eight 点 (dot) families and three 線
(line) families:

| Slug | Source keyword | Family |
|---|---|---|
| `goma` | 傍点 | 点 |
| `white-sesame` | 白ゴマ傍点 | 点 |
| `circle` | 丸傍点 | 点 |
| `white-circle` | 白丸傍点 | 点 |
| `double-circle` | 二重丸傍点 | 点 |
| `janome` | 蛇の目傍点 | 点 |
| `cross` | ばつ傍点 | 点 |
| `white-triangle` | 白三角傍点 | 点 |
| `wavy-line` | 波線 | 線 |
| `under-line` | 傍線 | 線 |
| `double-under-line` | 二重傍線 | 線 |

Each variant has a stable slug that the HTML renderer emits as a class
name (e.g. `<em class="aozora-bouten-goma">`). The 点/線 family boundary is
what [`mismatched_bouten_container`](diagnostics.md#mismatched-bouten-container)
checks for the range form below.

## Default rendering

aozora emits `<em class="aozora-bouten-<slug>">…</em>` so that an
external stylesheet can pick the visual treatment per variant.
Default CSS hooks live at the consumer side; the parser ships no
stylesheet of its own.

```html
<!-- 平和［＃「平和」に傍点］ -->
平和<em class="aozora-bouten aozora-bouten-goma aozora-bouten-right">平和</em>
```

(The redundant copy is intentional — the `［＃…］` indirection
*re-emits* the target wrapped in `<em>`, leaving the original run
in place. The HTML rendering matches what print Aozora Bunko output
does in practice.)

## Range form

To emphasise a run directly (rather than by quoting it), wrap it between a
**bare** opener and its matching closer — note there is **no** `ここから` /
`ここで` (those prefixes are for block layout / 太字 / 斜体, not 傍点):

```text
彼は［＃傍点］必ず［＃傍点終わり］来る
本文［＃二重傍線］乙［＃二重傍線終わり］
［＃左に傍線］丙［＃左に傍線終わり］
```

Renders inline as `<em>`:

```html
彼は<em class="aozora-bouten aozora-bouten-goma aozora-bouten-right">必ず</em>来る
```

The opener can be any variant keyword (`傍点`, `白丸傍点`, `二重傍線`, …), with
an optional `左に` prefix for left-side marks; the closer is the same
keyword plus `終わり`. The closer's **family** must match the opener's: a
点 opener (`［＃傍点］`) pairs with a 点 closer (`［＃傍点終わり］`), a 線 opener
(`［＃傍線］`) with a 線 closer (`［＃傍線終わり］`). A family mismatch fires
[`mismatched_bouten_container`](diagnostics.md#mismatched-bouten-container).

## AST shape

Both the indirect (`［＃「X」に傍点］`) and range (`［＃傍点］…［＃傍点終わり］`)
forms produce `Bouten` nodes:

```rust
pub struct Bouten<'src> {
    pub kind:     BoutenKind,            // one of 11 variants (点 / 線)
    pub target:   NonEmpty<Content<'src>>, // the emphasised run
    pub position: BoutenPosition,        // Right (default) | Left (左に…)
    pub consumed_predecessor: bool,      // whether it absorbed the run before it
}
```

`BoutenKind` is a flat enum (`BoutenKind::is_line` splits 点 from 線); see
the [rustdoc](../ref/api.md) for the exact variant list.

## See also

- [Notation overview](overview.md) — how this fits with the other
  inline annotations.
- [Diagnostics catalogue](diagnostics.md) —
  [`mismatched_bouten_container`](diagnostics.md#mismatched-bouten-container)
  and [`bouten_target_ambiguous`](diagnostics.md#bouten-target-ambiguous).
