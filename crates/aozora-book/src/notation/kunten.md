# Kunten / kaeriten (訓点・返り点)

**Kunten** are the marginal annotations Japanese readers add to
classical Chinese (漢文) source so that it can be read in Japanese
word order. aozora recognises **kaeriten** (返り点) — the reading-order
return marks — in their bracketed form. The recognised marks are:

- single: `レ`, `一`, `二`, `三`, `四`, `上`, `中`, `下`, `甲`, `乙`, `丙`, `丁`
- `Xレ` compounds: `一レ`, `二レ`, `三レ`, `上レ`, `中レ`, `下レ`
- 送り仮名: the parenthesised `（…）` form

(Re-reading marks — 再読文字 like 未 / 将 / 当 — and any other kunten that
do not match the above are carried as generic `［＃…］` annotations.)

A handful of late-Edo / Meiji Aozora Bunko works carry these. In real
source the marks sit between characters as `［＃…］` annotations:

```text
有［＃二］朋自遠方来［＃一］
```

## Notation forms

### Bracketed (the recognised form)

aozora recognises the **bracketed** form only — the mark in a `［＃…］`
annotation:

```text
有［＃二］朋自遠方来［＃一］
```

Renders as:

```html
有<sup class="aozora-kaeriten">二</sup>朋自遠方来<sup class="aozora-kaeriten">一</sup>
```

### Inline (not recognised)

A bare reading-mark glyph written directly between characters
(`有レ朋自遠方来`) is left as **plain text** — the parser cannot tell a
genuine 返り点 from an ordinary `一` / `上` / `レ` in running prose, which is
exactly why the bracketed form exists. Use `［＃…］` for any mark you want
recognised.

## Okurigana

Kunten 送り仮名 (reading-aid kana) use the parenthesised form, also inside a
`［＃…］` annotation:

```text
有［＃（リ）］
```

These are classified as kaeriten nodes but are **not** ladder marks (they
take no part in the [pairing check](diagnostics.md#bracketed-kaeriten-no-pair)).

## AST shape

The recognised marks (single `一 二 三 四 上 中 下 甲 乙 丙 丁 レ`, the `Xレ`
compounds, and `（…）` okurigana) all produce one node that stores the raw
mark text:

```rust
pub struct Kaeriten<'src> {
    pub mark: NonEmptyStr<'src>,   // the raw mark, e.g. "二" / "一レ" / "（リ）"
}
```

The renderer wraps it in `<sup class="aozora-kaeriten">…</sup>`. The
`bracketed_kaeriten_no_pair` / `kaeriten_outside_kanbun` checks classify the
mark's family and rank from this string at diagnostic time rather than
storing a typed enum.

## Diagnostics

| Code | Condition |
|---|---|
| [`kaeriten_outside_kanbun`](diagnostics.md#kaeriten-outside-kanbun) | A lone kaeriten in kana prose (conservative lookahead heuristic) |
| [`bracketed_kaeriten_no_pair`](diagnostics.md#bracketed-kaeriten-no-pair) | A rank-≥2 mark whose family base (`一` / `上` / `甲`) is absent from the document |

## See also

- [Notation overview](overview.md) — the orientation map for all the
  inline annotations.
