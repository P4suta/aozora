# Page & section breaks (改ページ・改丁)

Aozora Bunko inherits print conventions for page-level structure.
Four annotations split a work into pages, signatures, and openings:

| Notation | Renders as | Meaning |
|---|---|---|
| `［＃改ページ］` | `<div class="aozora-page-break"></div>` | Begin a new page |
| `［＃改丁］` | `<div class="aozora-section-break aozora-section-break-kaicho"></div>` | Begin a new 丁 (leaf / recto) |
| `［＃改段］` | `<div class="aozora-section-break aozora-section-break-kaidan"></div>` | Section break (smaller than a page) |
| `［＃改見開き］` | `<div class="aozora-section-break aozora-section-break-kaimihiraki"></div>` | Begin a new two-page spread |

All four are *self-contained* directives — no opener / closer pair,
no inner content. They appear on their own line in the source.

## AST shape

`［＃改ページ］` is its own borrowed-AST node; the three 段 / 丁 / 見開き
breaks share one `SectionBreak` node tagged by [`SectionKind`]:

```rust
// borrowed::AozoraNode variants
AozoraNode::PageBreak,                  // ［＃改ページ］
AozoraNode::SectionBreak(SectionKind),  // ［＃改丁 / 改段 / 改見開き］

pub enum SectionKind {
    Choho,   // 改丁
    Dan,     // 改段
    Spread,  // 改見開き
}
```

## Why distinct variants for each break flavour?

The flavours render to identical HTML *structure* (an empty `<div>`) but
different *class* hooks (`aozora-page-break`,
`aozora-section-break-{kaicho,kaidan,kaimihiraki}`). Keeping `PageBreak` separate
and tagging the section flavours with a `SectionKind` enum (rather than a
string) means:

- The renderer never plumbs the original notation through to the output,
  preserving the AST's role as a normalised IR.
- The compiler's exhaustiveness check guarantees every flavour has a
  render path.
- Tooling can *count* breaks of a specific flavour at the AST level
  without a string match.

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
| [`break_in_single_line_container`](diagnostics.md#break-in-single-line-container) | A page / section break sharing a line with a single-line container (or inside a warichu range), which drops it |

## See also

- [Indent containers](indent.md) — containers persist across breaks.
