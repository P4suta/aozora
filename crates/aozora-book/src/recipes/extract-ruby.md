# Extract ruby pairs

**Problem.** You want every ruby annotation in a document as
`(base, reading)` string pairs — to build a furigana glossary, audit
readings, or feed a dictionary.

## Solution

Walk `source_nodes()` (see [Walk the AST](walk-ast.md)), keep only the
`Ruby` nodes, and read each node's `base` and `reading`. Both are
`NonEmpty<Content>`; call `.get()` to get the `Content`, then
`.as_plain()` for the common case where the text carries no nested
constructs.

```rust
use aozora::{Document, AozoraNode, NodeRef};

fn main() {
    let source = "｜青梅《おうめ》街道を｜逢《お》う";
    let doc = Document::new(source);
    let tree = doc.parse();

    for sn in tree.source_nodes() {
        // Ruby is always an inline construct.
        if let NodeRef::Inline(AozoraNode::Ruby(ruby)) = sn.node {
            // `base` / `reading` are NonEmpty<Content>; `.get()` is the
            // Content, `.as_plain()` its text when there are no nested nodes.
            let base = ruby.base.get().as_plain().unwrap_or("<mixed>");
            let reading = ruby.reading.get().as_plain().unwrap_or("<mixed>");
            println!("{base}\t{reading}");
        }
    }
}
```

## Expected output

```text
青梅	おうめ
逢	お
```

## Notes

- **Why `NonEmpty`.** The parser only emits a `Ruby` node once both
  base and reading have content, so the fields are
  `NonEmpty<Content>` — an empty side is unrepresentable, and you
  never have to guard against it. `.get()` unwraps to the inner
  `Content`.

- **The `<mixed>` arm.** `Content::as_plain()` returns `None` when the
  run carries nested constructs (a gaiji reference or annotation inside
  the base, for instance). That is rare for readings but does happen
  for bases. To flatten those too, iterate the segments instead of
  bailing (`Segment` lives under the `syntax` module since it is not in
  the top-level re-export set):

  ```rust
  use aozora::syntax::borrowed::Segment;

  fn text_of(content: aozora::Content<'_>) -> String {
      let mut out = String::new();
      for seg in content.iter() {
          if let Segment::Text(s) = seg {
              out.push_str(s);
          }
          // Segment::Gaiji / Segment::Annotation carry non-plain payloads;
          // handle them here if your glossary needs them.
      }
      out
  }
  ```

  `Content::iter()` yields a `Segment` per logical run; the `Plain`
  case yields exactly one `Text` segment, so the loop is uniform.

- **`delim_explicit`.** `ruby.delim_explicit` records whether the
  source used the explicit `｜` base delimiter. It does not affect the
  base/reading text — see [the Ruby node chapter](../nodes/ruby.md) for
  why both source forms classify identically.

## See also

- Runnable example: **`just example walk_ast`**
  (`crates/aozora/examples/walk_ast.rs`) shows the full node walk this
  recipe narrows.
- [Walk the AST](walk-ast.md) — the general traversal.
- [Ruby node reference](../nodes/ruby.md) — the `Ruby` struct, the two
  source forms, and the rendered HTML.
- [Ruby notation](../notation/ruby.md) — the `｜青梅《おうめ》`
  syntax itself.
