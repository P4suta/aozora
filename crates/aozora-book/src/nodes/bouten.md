# NodeKind::Bouten

Wire tag: `bouten` — emphasis dots / sidelines over a target span.

## Source examples

```text
青空に［＃「青空」に傍点］
青空に［＃「青空」に丸傍点］
```

The bracketed annotation refers backwards to the literal text
quoted with `「…」`, so the parser resolves the target by string
match against the preceding line(s).

## Rendered HTML

```html
<em class="aozora-bouten aozora-bouten-goma aozora-bouten-right">青空</em>に
```

The two trailing class slots carry the bouten kind (`goma`,
`circle`, `wavy-line`, …) and the position (`right` for vertical
text, `left` for the rare under-side variant).

## Serialize output

Round-trips to the explicit `［＃「target」に<kind>傍点］` form.

## AST shape

```rust,ignore
pub struct Bouten<'src> {
    pub kind: BoutenKind,
    pub target: NonEmpty<Content<'src>>,
    pub position: BoutenPosition,
}
```

`BoutenKind` enumerates the 11 visual variants (Goma, WhiteSesame,
Circle, …); `BoutenPosition` is `Right` (default for vertical text)
or `Left`.

## When emitted

Phase 3 sees `［＃「QUOTE」に <slug>傍点］` / `［＃「QUOTE」に <slug>傍線］`,
walks back through the recent text to find QUOTE, and emits the
node with the matched span.

## Diagnostics

- `aozora::lex::unclosed_bracket` — annotation `［＃` opened with no
  matching `］`.
- `Annotation` (fallback) — quote target unresolved.

## Related kinds

- [Annotation](annotation.md) — fallback when the target cannot be
  matched.
