# Page & section breaks (改ページ・改丁)

Aozora Bunko inherits print conventions for page-level structure.
Four annotations split a work into pages, signatures, and openings:

| Notation | Renders as | Meaning |
|---|---|---|
| `［＃改ページ］` | `<div class="aozora-page-break"/>` | Begin a new page |
| `［＃改丁］` | `<div class="aozora-page-break aozora-recto"/>` | Begin a new recto (right-hand) page |
| `［＃改見開き］` | `<div class="aozora-page-break aozora-spread"/>` | Begin a new two-page spread |
| `［＃改段］` | `<div class="aozora-section-break"/>` | Section break (smaller than a page) |

All four are *self-contained* directives — no opener / closer pair,
no inner content. They appear on their own line in the source.

## AST shape

```rust
pub enum Break {
    Page,
    PageRecto,        // 改丁
    PageSpread,       // 改見開き
    Section,          // 改段
}

pub struct BreakNode {
    pub kind: Break,
    pub span: Span,
}
```

## Why distinct variants for each break flavour?

The four flavours render to identical HTML *structure* (an empty
`<div>`) but different *class* hooks. Collapsing them to a single
variant with a string tag would:

- Force the renderer to plumb the original notation through to the
  output, defeating the AST's role as a normalised IR.
- Lose the type-system check that every break flavour has a render
  path — clippy's exhaustiveness lint catches the bug at compile time.
- Make it impossible to *count* page breaks of a specific flavour at
  the AST level without a string match.

The 4-variant enum is 1 byte plus discriminant — no real cost over
the alternative.

## Composition with other annotations

Breaks unconditionally close any open inline annotation (ruby, bouten,
tcy) at their line. They do **not** close container directives
(字下げ, 地付き, etc.) — those persist across page boundaries, which
matches print typography.

```text
［＃ここから2字下げ］
　第一節
［＃改ページ］
　第二節 (still 2字下げ)
［＃ここで字下げ終わり］
```

## Diagnostics

| Code | Condition |
|---|---|
| [`W0008`](diagnostics.md#W0008) | Page break inside a single-line container (drops the container) |

## See also

- [Indent containers](indent.md) — containers persist across breaks.
