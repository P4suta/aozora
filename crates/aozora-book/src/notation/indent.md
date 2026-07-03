# Indent & align containers (字下げ)

Aozora Bunko uses paired `［＃ここから…］` / `［＃ここで…終わり］`
brackets to delimit blocks of text with custom layout. The block
container families aozora recognises:

| Family | Opener | Closer | Effect |
|---|---|---|---|
| 字下げ (indent) | `［＃ここから2字下げ］` | `［＃ここで字下げ終わり］` | Indent every line by N full-width chars |
| 地付き / 地上げ (align-end) | `［＃ここから地付き］` / `［＃ここから地から2字上げ］` | `［＃ここで地付き終わり］` | Flush right (vertical: 地 = ground = bottom) |
| 罫囲み (boxed) | `［＃罫囲み］` | `［＃罫囲み終わり］` | Draw a rule frame around the block |

The HTML renderer maps them to `<div class="aozora-container …">` wrappers.
Two more container kinds are **inline**, not block: 割り注
(`［＃割り注］…［＃割り注終わり］`) and the 傍点 / 傍線 range form
(`［＃傍点］…［＃傍点終わり］`, see [bouten](bouten.md)).

## Single-line forms

The 字下げ / 地付き / 地上げ directives also have a **single-line** form
(no `ここから` prefix, no closer) that applies to the rest of the line:

```text
　［＃地付き］平和への誓い
```

In the owned AST a single-line directive is a **zero-width marker**
node (`Node::Indent` / `AlignEnd`), not a wrapping container — it
renders as an empty span and the following text stays a sibling:

```html
<span class="aozora-align-end aozora-align-end-0" data-offset="0"></span>平和への誓い
```

A page / section break sharing the line with such a marker drops it —
see [`break_in_single_line_container`](diagnostics.md#break-in-single-line-container).

## AST shape

A paired block container is one `Container` node tagging the wrapped
children (the lexer splices the enclosed siblings under it during
post-processing); single-line forms and breaks are leaf nodes:

```rust,ignore
pub struct Container {
    pub kind: ContainerKind,
}

pub enum ContainerKind {
    Indent { amount: u8 },    // ［＃ここからN字下げ］
    AlignEnd { offset: u8 },  // ［＃ここから地付き / 地からN字上げ］
    Framed,                // ［＃罫囲み］
    Warichu,                  // ［＃割り注］           (inline)
    BoutenRange { kind: BoutenKind, position: BoutenPosition }, // ［＃傍点］… (inline)
}
```

## Why a small flat enum?

`ContainerKind` is closed by spec. A flat `enum` (vs a trait object or
string tag) gives the parser O(1) variant dispatch in the classify stage
and the renderer's HTML walk, *and* lets the compiler's exhaustiveness
check enforce that every variant has a render path. The payloads are tiny
(`u8` / `BoutenKind` / `BoutenPosition`), so the whole enum stays within a
few bytes — pinned by the `container_kind_is_copy_and_fits_in_a_word`
assertion.

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
fire diagnostic
[`aozora::lex::mismatched_container_close`](diagnostics.md#mismatched-container-close)
and the parser auto-closes the offending opener at the closer's position.
The check compares container *families*, so closing a `2字下げ` opener
with a plain `字下げ終わり` (both `indent`) is fine — only a different
family (indent vs align-end vs 罫囲み vs 割り注) is flagged.

## Why containers, not stack-based push/pop tokens?

The spec describes these as opener / closer brackets, but the natural
implementation in Rust is a *recursive container node*. That choice:

- Lets the renderer walk the tree once with a single match on
  `ContainerKind`, instead of maintaining a render-time stack.
- Surfaces shape errors (mismatched closers, dangling openers) at
  parse time — the lexer's classify stage already has all the
  information to decide.
- Makes the canonical-source pass trivial (each container
  prints its opener, walks its children, prints its closer).

The trade-off is one extra entry per container — the `children` live
as a `SegRange` handle into the tree's shared segment pool, not a
separate allocation. The pool is a single growable `Vec`, so
appending a child run is an amortised O(1) push with no per-node
allocation.

## See also

- [Architecture → Owned AST & NodeStore](../arch/arena.md) — how
  container child slices are laid out.
- [Diagnostics → `aozora::lex::mismatched_container_close`](diagnostics.md#mismatched-container-close)
  — mismatched closer.
