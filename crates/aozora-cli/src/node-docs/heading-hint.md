# NodeKind::HeadingHint

Wire tag: `headingHint` — forward-reference heading hint
(`［＃「target」は中見出し］`).

## Source examples

```text
序章
［＃「序章」は中見出し］
```

The hint refers to a quoted target string in the preceding line(s);
downstream renderers pick this up as "promote the matched run to a
heading."

## Rendered HTML

The marker itself emits no visible content; renderers that *honour*
the hint elevate the previously-matched span to a `<h2>` /
`<h3>` retroactively. The default HTML renderer in `aozora-render`
emits a structural marker comment.

## Serialize output

Round-trips to `［＃「target」は<level>見出し］`.

## AST shape

```rust,ignore
pub struct HeadingHint<'src> {
    pub level: u8,
    pub target: NonEmptyStr<'src>,
}
```

`level` follows the Aozora convention: 1=大見出し, 2=中見出し,
3=小見出し.

## When emitted

Phase 3 matches the directive and records the level + target. Empty
target is rejected and falls through to plain text.

## Diagnostics

None on well-formed input.

## Related kinds

- [AozoraHeading](aozora-heading.md) — direct heading-marker variant.
