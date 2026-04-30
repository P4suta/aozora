# Kunten / kaeriten (訓点・返り点)

**Kunten** are the marginal annotations Japanese readers add to
classical Chinese (漢文) source so that it can be read in Japanese
word order. The two categories aozora handles:

1. **Kaeriten** (返り点) — reading-order marks inserted between
   characters: `レ`, `一`, `二`, `三`, `上`, `中`, `下`, `甲`, `乙`,
   `天`, `地`, `人`.
2. **Saidoku-moji** (再読文字) — characters that are read twice with
   different glosses (e.g. 未, 将, 当).

A handful of late-Edo / Meiji Aozora Bunko works carry these. The
notation:

```text
有﹅レ朋﹅自﹅遠﹅方﹅来
```

…where `﹅` stands in for the actual kaeriten character. In real
source the marks are interleaved between characters using either the
direct character or a `［＃…］` annotation:

```text
有［＃二］朋自遠方来［＃一］
```

## Notation forms

### Inline (preferred in modern works)

The kaeriten character is inserted directly between the source
characters:

```text
有レ朋自遠方来
```

Renders as:

```html
有<span class="aozora-kaeriten" data-aozora-kaeriten="レ">レ</span>朋自遠方来
```

### Bracketed (older works)

```text
有［＃二］朋自遠方来［＃一］
```

Renders as:

```html
有<span class="aozora-kaeriten" data-aozora-kaeriten="二">二</span>朋自遠方来<span class="aozora-kaeriten" data-aozora-kaeriten="一">一</span>
```

The bracketed form is useful when the kaeriten character would
otherwise be ambiguous with the surrounding text (e.g. a real `一`
that is *not* a reading mark).

## Saidoku-moji

```text
未［＃「未」に二の字点］
```

The 二の字点 / 一二点 prefix tells the renderer that the preceding
character is read twice. aozora emits a `data-aozora-saidoku` data
attribute on the wrapper.

## AST shape

```rust
pub struct Kaeriten<'src> {
    pub mark: KaeritenKind,    // Re | Ichi | Ni | San | Jou | Chuu | Ge | Kou | Otsu | Ten | Chi | Jin
    pub form: KaeritenForm,    // Inline | Bracketed
    pub span: Span,
}

pub struct Saidoku<'src> {
    pub target: &'src str,     // the character being re-read
    pub gloss:  &'src str,     // the second reading
    pub span:   Span,
}
```

## Why a flat enum, not just `&str`?

The 13 kaeriten kinds form a closed set fixed by the spec — there
will never be a 14th. A `KaeritenKind` enum lets the renderer match
exhaustively (the compiler catches unhandled variants), and pins the
`data-aozora-kaeriten` attribute value to a stable slug rather than
the literal source character. That matters because the inline form
uses the actual `一` / `二` / `上` / … glyphs, which are also valid
plain text — the enum lets the AST distinguish "a `一` that's a
kaeriten" from "the digit one in the running text".

## Diagnostics

| Code | Condition |
|---|---|
| [`W0007`](diagnostics.md#W0007) | Kaeriten outside a 漢文-like context (lookahead heuristic) |
| [`E0009`](diagnostics.md#E0009) | Bracketed kaeriten with no matching pair |

## See also

- [Notation overview](overview.md) — the orientation map for all the
  inline annotations.
