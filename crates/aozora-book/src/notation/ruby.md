# Ruby (`｜青梅《おうめ》`)

Ruby is a pronunciation gloss attached to a run of base text. In
青空文庫 source it appears in two shapes:

```text
｜青梅《おうめ》            ← explicit-base form
青梅《おうめ》              ← implicit-base form (auto-detect)
```

Both forms render the same HTML:

```html
<ruby>青梅<rt>おうめ</rt></ruby>
```

## Explicit base (`｜…《…》`)

The full-width vertical bar `｜` (U+FF5C) marks the *start* of the
base text; `《…》` (U+300A / U+300B) wraps the reading. The base
runs from `｜` to the `《`. Use this form when:

- The base contains characters that the auto-detect heuristic would
  otherwise skip (kana, ASCII letters, mixed scripts).
- The boundary between base and surrounding text is ambiguous.

```text
｜山田《やまだ》さん         → <ruby>山田<rt>やまだ</rt></ruby>さん
｜HTTP《ハイパー・テキスト》 → <ruby>HTTP<rt>ハイパー・テキスト</rt></ruby>
```

## Implicit base

When `《…》` follows a run of kanji *without* a leading `｜`, the
parser auto-detects the base by scanning backwards through the kanji
run. The auto-detect terminates at the first non-kanji character
(kana, punctuation, ASCII, full-width digit).

```text
青梅《おうめ》     → <ruby>青梅<rt>おうめ</rt></ruby>
お青梅《おうめ》   → お<ruby>青梅<rt>おうめ</rt></ruby>
```

The "kanji" predicate is **CJK Unified Ideographs** + **CJK
Compatibility Ideographs** + **CJK Unified Ideographs Extension A–F**
+ the iteration mark `々`. JIS X 0213 plane-2 ideographs not in
Unicode are represented as gaiji references (see
[Gaiji](gaiji.md)) and likewise terminate the auto-detect.

## Empty reading

`｜青梅《》` supplies a base but an empty reading. The lexer emits
[`aozora::lex::empty_ruby_reading`](diagnostics.md#empty-ruby-reading)
(an `Error`) and the construct degrades to plain text — no `Ruby` node is
built.

The implicit-base form silently skips a `《》` with empty contents — the
parser can't be sure a base was intended, so it treats the bare `《》` as
literal text and stays silent.

## Nested ruby (forbidden)

The spec disallows ruby inside ruby. A reading whose body opens another
`《…》` (e.g. `｜漢《か《ん》じ》`) fires
[`aozora::lex::nested_ruby`](diagnostics.md#nested-ruby); the outer ruby
is still parsed best-effort. (An *adjacent* `《《…》》` is a different
construct — [double-bracket bouten](bouten.md) — not nested ruby.)

## AST shape

```rust,ignore
pub struct Ruby<'src> {
    pub base:    NonEmpty<Content<'src>>,  // never empty
    pub reading: NonEmpty<Content<'src>>,  // never empty
    pub side:    RubySide,                 // Right for ｜《》/implicit; Left 左ルビ
}
```

`base` and `reading` are [`Content`] (a `Plain(&str)` fast path or a
`Segments` run carrying nested gaiji / annotations), wrapped in
`NonEmpty` so an empty payload is unrepresentable — the classify stage
only emits a `Ruby` once both sides have content (an empty reading takes
the [empty-reading](#empty-reading) path instead). The node does not
record which surface form the author used: `to_source` emits the
**canonical bare** form `base《reading》` and only adds an explicit `｜`
when a bare reading would re-parse to a *different* base (a base that
mixes character classes, or one preceded by another base char / `｜`) —
see ADR 0002/0003. `to_source_verbatim` preserves the author's exact
bytes, including any redundant `｜`.

## Edge cases

| Input | Output |
|---|---|
| `青梅《おうめ》` | `<ruby>青梅<rt>おうめ</rt></ruby>` |
| `｜青梅《おうめ》` | `<ruby>青梅<rt>おうめ</rt></ruby>` (canonical-equivalent) |
| `｜山田《やまだ》` | `<ruby>山田<rt>やまだ</rt></ruby>` |
| `｜HTTP《ハイパー・テキスト》` | `<ruby>HTTP<rt>ハイパー・テキスト</rt></ruby>` |
| `お青梅《おうめ》` | `お<ruby>青梅<rt>おうめ</rt></ruby>` (auto-detect skips kana) |
| `1青梅《おうめ》` | `1<ruby>青梅<rt>おうめ</rt></ruby>` (auto-detect skips digit) |
| `｜青梅《》` | plain text + [`empty_ruby_reading`](diagnostics.md#empty-ruby-reading) |
| `《おうめ》` | literal text (no preceding kanji to anchor) |
| `｜漢《か《ん》じ》` | best-effort ruby + [`nested_ruby`](diagnostics.md#nested-ruby) |

## See also

- [Bouten / bousen](bouten.md) — emphasis annotations that share the
  `「X」に…` indirection idiom.
- [Architecture → Lexer (sanitize → tokenize → pair → classify)](../arch/lexer.md)
  — where ruby recognition fits in the classifier pipeline.
