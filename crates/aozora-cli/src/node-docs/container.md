# NodeKind::Container

Inspect tag: `container` — paired-container wrapping
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
<div class="aozora-container aozora-container-indent aozora-container-indent-2" data-amount="2">
  ...
</div>
```

The wrapping div always carries `aozora-container`, then a class for
its kind, then any modifier the kind takes. Structural values repeat on
`data-*`. Run `aozora spec slugs` for the ［＃…］ forms that produce
each one.

## Source output

Round-trips to the explicit-paired directive form.

## When emitted

The pair stage pairs the `［＃ここから…］` / `［＃ここで…終わり］` openers
and closers; the classify stage's `BlockOpen` / `BlockClose` events
project to this variant.

## Diagnostics

`unclosed_bracket` for unbalanced opens.

## Related kinds

- `containerOpen` — `NodeRef` projection of the
  open boundary.
- `containerClose` — `NodeRef` projection of the
  close boundary.
- `indent`, `alignEnd`,
  `warichu` — single-line counterparts.
