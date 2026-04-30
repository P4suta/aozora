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

`｜青梅《》` is a parse error. The lexer emits diagnostic
[`E0001`](diagnostics.md#E0001) ("ruby reading mismatch: target spans
N chars but `｜《》` reading is empty") and the node is dropped
from the AST.

The implicit-base form silently skips a `《》` with empty contents —
that combination cannot have arisen from valid markup, so the parser
treats the bare `《》` as literal text.

## Nested ruby (forbidden)

The spec disallows ruby inside ruby. Sources with `｜青梅《｜お《お》うめ》`
are rejected with diagnostic [`E0002`](diagnostics.md#E0002).

## AST shape

```rust
pub struct Ruby<'src> {
    pub target:  &'src str,   // borrowed from source
    pub reading: &'src str,   // borrowed from source
    pub span:    Span,        // byte range in the source
    pub explicit_base: bool,  // true if the input used the ｜…《…》 form
}
```

Both `target` and `reading` are `&str` slices into the
`Document`-owned source — no allocation, no copy. Re-emitting
canonical form is exactly:

```rust
match (ruby.explicit_base, ruby.target, ruby.reading) {
    (true,  t, r) => format!("｜{t}《{r}》"),
    (false, t, r) => format!("{t}《{r}》"),
}
```

## Edge cases

| Input | Output |
|---|---|
| `青梅《おうめ》` | `<ruby>青梅<rt>おうめ</rt></ruby>` |
| `｜青梅《おうめ》` | `<ruby>青梅<rt>おうめ</rt></ruby>` (canonical-equivalent) |
| `｜山田《やまだ》` | `<ruby>山田<rt>やまだ</rt></ruby>` |
| `｜HTTP《ハイパー・テキスト》` | `<ruby>HTTP<rt>ハイパー・テキスト</rt></ruby>` |
| `お青梅《おうめ》` | `お<ruby>青梅<rt>おうめ</rt></ruby>` (auto-detect skips kana) |
| `1青梅《おうめ》` | `1<ruby>青梅<rt>おうめ</rt></ruby>` (auto-detect skips digit) |
| `｜青梅《》` | parse error `E0001` |
| `《おうめ》` | literal text (no preceding kanji to anchor) |
| `｜青梅《｜お《お》うめ》` | parse error `E0002` |

## See also

- [Bouten / bousen](bouten.md) — emphasis annotations that share the
  `「X」に…` indirection idiom.
- [Architecture → Seven-phase lexer](../arch/lexer.md) — where ruby
  recognition fits in the classifier pipeline.
