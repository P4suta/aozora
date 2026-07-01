# Node reference

aozora exposes 19 [`NodeKind`][nodekind] variants. Each is documented
on its own page with source examples, the rendered HTML, the
source round-trip output, the in-memory AST shape, and the
diagnostics it can fire alongside.

The page layout matches the `aozora explain <kind>` CLI subcommand:
once you find the variant in the table, the deep dive is one click —
or one shell invocation — away.

| Variant | JSON tag | Notation |
| --- | --- | --- |
| [Ruby](ruby.md) | `ruby` | `｜base《reading》` |
| [Bouten](bouten.md) | `bouten` | `［＃「target」に傍点］` |
| [CombineUpright](tate-chu-yoko.md) | `tateChuYoko` | `［＃「12」は縦中横］` |
| [Gaiji](gaiji.md) | `gaiji` | `※［＃...、第3水準1-85-54］` |
| [Indent](indent.md) | `indent` | `［＃2字下げ］` |
| [AlignEnd](align-end.md) | `alignEnd` | `［＃地から2字上げ］` |
| [Warichu](warichu.md) | `warichu` | `［＃割り注］...` |
| [Framed](keigakomi.md) | `keigakomi` | `［＃罫囲み］` |
| [PageBreak](page-break.md) | `pageBreak` | `［＃改ページ］` |
| [SectionBreak](section-break.md) | `sectionBreak` | `［＃改丁］` |
| [Heading](aozora-heading.md) | `heading` | `［＃見出し］` |
| [HeadingHint](heading-hint.md) | `headingHint` | `［＃「対象」は中見出し］` |
| [Illustration](sashie.md) | `sashie` | `［＃挿絵（path.png）入る］` |
| [Kaeriten](kaeriten.md) | `kaeriten` | `［＃返り点 一・二］` |
| [Directive](annotation.md) | `annotation` | `［＃任意のコメント］` |
| [AngleQuote](angle-quote.md) | `angleQuote` | `≪重要≫` → `《重要》` |
| [Container](container.md) | `container` | `［＃ここから...］...［＃ここで...終わり］` |
| [ContainerOpen](container-open.md) | `containerOpen` | (NodeRef projection) |
| [ContainerClose](container-close.md) | `containerClose` | (NodeRef projection) |

## How to read these pages

Every node page follows the same skeleton:

| Section | Content |
| --- | --- |
| Source examples | One or two minimal Aozora-notation strings that produce this variant. |
| Rendered HTML | What `Document::new(src).parse().to_html()` emits. |
| Source output | What `to_source()` emits — typically the canonical form of the source. |
| AST shape | The owned-AST (`NodeOwned`) struct fields the variant carries. |
| When emitted | Classify-stage rule that produces this variant. |
| Diagnostics | Codes that may accompany this variant. |
| Related kinds | Cross-links to neighbours (`Bouten` ↔ `Bousen`, `Indent` ↔ `Container::Indent`, etc.). |

`#[non_exhaustive]` on `NodeKind`: a future minor release adding a
new variant lands here without a breaking change. Downstream
consumers that match on `NodeKind` exhaustively must include a `_`
arm.

[nodekind]: https://docs.rs/aozora/latest/aozora/enum.NodeKind.html
