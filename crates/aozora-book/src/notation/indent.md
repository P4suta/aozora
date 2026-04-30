# Indent & align containers (字下げ)

Aozora Bunko uses paired `［＃ここから…］` / `［＃ここで…終わり］`
brackets to delimit blocks of text with custom layout. The five
families:

| Family | Opener | Closer | Effect |
|---|---|---|---|
| 字下げ (indent) | `［＃ここから2字下げ］` | `［＃ここで字下げ終わり］` | Indent every line by N full-width chars |
| 地付き (right-flush) | `［＃ここから地付き］` | `［＃ここで地付き終わり］` | Flush right (vertical: 地 = ground = bottom) |
| 地寄せ (right-align with margin) | `［＃ここから2字下げ、地寄せ］` | `［＃ここで字下げ終わり］` | Right-align with N-char inset |
| 字詰め (line-length) | `［＃ここから30字詰め］` | `［＃ここで字詰め終わり］` | Force a line length of N chars |
| 中央揃え | `［＃ここから中央揃え］` | `［＃ここで中央揃え終わり］` | Centre each line |

aozora parses every variant; the HTML renderer maps them to a
`<div class="aozora-indent-N">` / `aozora-align-end` / etc. wrapper.

## Single-line forms

Some directives apply only to the next single line and don't need a
closer:

```text
　［＃地付き］平和への誓い
```

Renders as:

```html
<div class="aozora-align-end">平和への誓い</div>
```

## AST shape

```rust
pub struct Container<'src> {
    pub kind:    ContainerKind,
    pub indent:  Option<u8>,      // 字 count for indent variants
    pub form:    ContainerForm,   // SingleLine | Block
    pub children: &'src [AozoraNode<'src>],
    pub span:    Span,
}

pub enum ContainerKind {
    Indent,
    AlignEnd,
    AlignEndWithIndent,
    LineLength,
    Centre,
    /// Composite: indent + align-end on a single block.
    Composite { indent: u8, align: ContainerAlign },
    /// Bouten / 縦中横 / 鎖線 / 罫囲み container forms.
    Emphasis(EmphasisKind),
    /// Spec-listed but not present in maintained corpus.
    Unknown,
}
```

## Why a small flat enum?

`ContainerKind` is closed by spec. A flat `enum` (vs a trait object
or string tag) gives the parser O(1) variant dispatch in the lexer's
classify phase and the renderer's HTML walk, *and* lets clippy's
exhaustiveness check enforce that every variant has a render path.

The `Composite` variant is the one place we *don't* extend the enum
horizontally — composite indent+align combinations would explode the
enum to ~30 variants, most of which never appear in real corpus. A
nested struct with a sub-enum keeps the variant count finite while
staying matchable.

`large_enum_variant` clippy lint: `Container::Composite` is the
largest variant at 4 bytes; the others are ≤ 2 bytes. The variant
data is tiny enough that boxing would add a pointer chase for no
real layout win — see the `[workspace.lints.clippy]
large_enum_variant = "allow"` carve-out in `Cargo.toml`.

## Composition

Containers nest:

```text
［＃ここから2字下げ］
　通常の段落。
　［＃ここから地付き］
　　右寄せの行。
　［＃ここで地付き終わり］
　通常に戻る。
［＃ここで字下げ終わり］
```

Renders as nested divs:

```html
<div class="aozora-indent-2">
通常の段落。
<div class="aozora-align-end">
右寄せの行。
</div>
通常に戻る。
</div>
```

Mismatched closers (e.g. `［＃ここから地付き］` … `［＃ここで字下げ終わり］`)
fire diagnostic [`E0005`](diagnostics.md#E0005) and the parser
auto-closes the offending opener at the closer's position.

## Why containers, not stack-based push/pop tokens?

The spec describes these as opener / closer brackets, but the natural
implementation in Rust is a *recursive container node*. That choice:

- Lets the renderer walk the tree once with a single match on
  `ContainerKind`, instead of maintaining a render-time stack.
- Surfaces shape errors (mismatched closers, dangling openers) at
  parse time — the lexer's classify phase already has all the
  information to decide.
- Makes the canonical-serialise pass trivial (each container
  prints its opener, walks its children, prints its closer).

The trade-off is one extra heap touch per container — a single
`bumpalo` slice for `children`. The arena is already hot, so the cost
is negligible (`bumpalo` returns aligned pointers in O(1) bumps).

## See also

- [Architecture → Borrowed-arena AST](../arch/arena.md) — how
  container child slices are laid out in the arena.
- [Diagnostics → `E0005`](diagnostics.md#E0005) — mismatched closer.
