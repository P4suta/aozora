# Notation overview

青空文庫記法 is a small, line-oriented annotation language layered
*inside* a plain-text Japanese document. Authors mark up the text in
two distinct registers:

1. **Inline markers** — single-character sigils (`｜`, `《`, `》`, `※`)
   that fence inline annotations directly inside the prose.
2. **Block annotations** — `［＃…］` brackets containing a Japanese
   directive in natural language ("ここから2字下げ", "「X」に傍点", …)
   that act as openers, closers, or self-contained directives.

aozora recognises every annotation that survives in real Aozora Bunko
sources — the volunteer corpus has ~17 000 works in active rotation,
and the parser is exercised against the entire archive in CI as part
of the [corpus sweep](../perf/corpus.md).

## Notations covered

| Chapter | What it marks |
|---|---|
| [Ruby](ruby.md) | Pronunciation glosses (`｜青梅《おうめ》`, `青梅《おうめ》`). |
| [Bouten / bousen](bouten.md) | Emphasis dots and lines: 傍点 (sesame, white sesame, filled circle, open circle, …) and 傍線 (single, double, dashed, …). |
| [縦中横](tcy.md) | Horizontally-set runs inside vertical text (`［＃「数字」は縦中横］`). |
| [Gaiji](gaiji.md) | Out-of-Shift_JIS character references (`※［＃…、第3水準1-85-54］`) and accented-Latin decomposition. |
| [Kunten](kunten.md) | 漢文 reading marks: 返り点 (`レ`, `一`, `二`, `上`, `中`, `下`), 再読文字, 送り仮名. |
| [Indent containers](indent.md) | `［＃ここから2字下げ］… ［＃ここで字下げ終わり］` and the geji / 地付き / 地寄せ family. |
| [Page & section breaks](breaks.md) | 改ページ, 改丁, 改見開き, 改段. |
| [Diagnostics](diagnostics.md) | The catalogue of structured diagnostics the parser emits. |

## Spec source of truth

The authoritative spec lives at
<https://www.aozora.gr.jp/annotation/index.html>. A snapshot is
vendored at [`docs/specs/aozora/`](https://github.com/P4suta/aozora/tree/main/docs/specs/aozora)
in the repo so that every page in this handbook can link to a stable
fragment (the upstream HTML reorganises occasionally; the snapshot
shields chapter cross-references from rot).

When this handbook says "the spec says X", that means *that snapshot*.
Where the live spec drifts, we update the snapshot, then update the
parser, then update this handbook — in that order.

## How a sample input looks

```text
｜青梅《おうめ》街道を歩いて、※［＃「魚＋師のつくり」、第3水準1-94-37］を見た。
［＃ここから2字下げ］
　［＃「平和」に傍点］という言葉は、もう古い。
［＃ここで字下げ終わり］
［＃改ページ］
```

That single sample exercises ruby, gaiji, indent containers, bouten,
and a page break. The parser turns it into a flat node stream — see
the per-chapter pages for the exact AST shapes.

## Notation we deliberately omit

Aozora Bunko's spec mentions a handful of annotations that don't
appear in the maintained corpus:

- **Image references** beyond `［＃挿絵］` — covered up to the
  caption, no actual image rendering.
- **キャプション alignment** edge cases that the spec lists but no
  active work uses (verified against the corpus sweep).

These are recognised as `Container::Unknown` with a
[`W0010`](diagnostics.md#W0010) advisory diagnostic. Adding full
support is a one-PR job once a real corpus document needs it.
