# NodeKind::Container

Wire tag: `container` — paired-container wrapping
(`［＃ここから...］...［＃ここで...終わり］`).

## Source examples

```text
［＃ここから2字下げ］
　第一節
　第二節
［＃ここで字下げ終わり］

［＃罫囲み］
本文
［＃罫囲み終わり］

［＃地から3字上げ］
寄付者一覧
［＃字上げ終わり］
```

## Rendered HTML

```html
<div class="aozora-container-indent" data-amount="2">
  ...
</div>
```

The wrapping div carries the kind-specific class
(`aozora-container-indent`, `aozora-container-warichu`,
`aozora-container-keigakomi`, `aozora-container-align-end`) plus
any structural data (indent amount, align offset) on `data-*`.

## Serialize output

Round-trips to the explicit-paired directive form.

## AST shape

```rust,ignore
pub struct Container {
    pub kind: ContainerKind,
}

pub enum ContainerKind {
    Indent { amount: u8 },
    Warichu,
    Keigakomi,
    AlignEnd { offset: u8 },
}
```

The `Container` payload appears wrapping the *content* — the actual
walker driver fires `visit_container_open` on enter and
`visit_container_close` on exit so renderers wrap the body cleanly.

## When emitted

Phase 2 pairs the `［＃ここから…］` / `［＃ここで…終わり］` openers
and closers; Phase 3's `BlockOpen` / `BlockClose` events project to
this variant.

## Diagnostics

`unclosed_bracket` for unbalanced opens.

## Related kinds

- [ContainerOpen](container-open.md) — `NodeRef` projection of the
  open boundary.
- [ContainerClose](container-close.md) — `NodeRef` projection of the
  close boundary.
- [Indent](indent.md), [AlignEnd](align-end.md), [Warichu](warichu.md),
  [Keigakomi](keigakomi.md) — single-line counterparts.
