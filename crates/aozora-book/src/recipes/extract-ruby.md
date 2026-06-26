# Extract ruby pairs

**Problem.** You want every ruby annotation in a document as
`(base, reading)` string pairs — to build a furigana glossary, audit
readings, or feed a dictionary.

## Solution

Walk `source_nodes()` (see [Walk the AST](walk-ast.md)), keep only the
`Ruby` nodes, and read each node's `base` and `reading`. Both are
`ContentRange` handles into the tree's `NodeStore`; resolve each with
`store.content_range_as_plain(..)` for the common case where the text
carries no nested constructs.

```rust
# extern crate aozora;
use aozora::{Document, NodeOwned, NodeRefOwned};

fn main() {
    let source = "｜青梅《おうめ》街道を｜逢《お》う";
    let doc = Document::new(source);
    let tree = doc.parse();
    let store = &tree.lex_output().store;

    for sn in tree.source_nodes() {
        // Ruby is always an inline construct.
        if let NodeRefOwned::Inline(NodeOwned::Ruby(ruby)) = sn.node {
            // `base` / `reading` are ContentRange handles; resolve them
            // against the store. `content_range_as_plain` is Some for the
            // common no-nested-construct case, None for mixed content.
            let base = store.content_range_as_plain(ruby.base).unwrap_or("<mixed>");
            let reading = store.content_range_as_plain(ruby.reading).unwrap_or("<mixed>");
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

- **Why a node only appears once it has content.** The parser only
  emits a `Ruby` node once both base and reading have content, so a
  `Ruby` node always carries a non-empty `base` and `reading` — an empty
  side is unrepresentable, and you never have to guard against it. The
  fields are `ContentRange` handles resolved against the tree's
  `NodeStore` (reached via `tree.lex_output().store`).

- **The `<mixed>` arm.** `NodeStore::content_range_as_plain()` returns
  `None` when the run carries nested constructs (a gaiji reference or
  annotation inside the base, for instance). That is rare for readings
  but does happen for bases. To flatten those too, walk the resolved
  content / segment runs instead of bailing (the owned `ContentOwned` /
  `SegmentOwned` types live under the `syntax::owned` module):

  ```rust
  # extern crate aozora;
  use aozora::syntax::owned::{ContentOwned, NodeStore, SegmentOwned};
  use aozora::syntax::owned::ContentRange;

  fn text_of(range: ContentRange, store: &NodeStore) -> String {
      let mut out = String::new();
      for &content in store.resolve_content_range(range) {
          match content {
              ContentOwned::Plain(id) => out.push_str(store.resolve_str(id)),
              ContentOwned::Segments(seg_range) => {
                  for seg in store.resolve_seg_range(seg_range) {
                      if let SegmentOwned::Text(id) = seg {
                          out.push_str(store.resolve_str(*id));
                      }
                      // SegmentOwned::Gaiji / Directive carry non-plain
                      // payloads; handle them here if your glossary needs them.
                  }
              }
              _ => {}
          }
      }
      out
  }
  ```

  `resolve_content_range` yields the run's `ContentOwned` entries; a
  `Plain` resolves to one interned string, a `Segments` run to a
  `SegmentOwned` sequence.

- **`side`.** `ruby.side` is `RubySide::Right` for the standard
  `｜base《reading》` / implicit forms and `RubySide::Left` for the
  `［＃「X」の左に「Y」のルビ］` saidoku building block; a furigana
  glossary usually keeps only `Right` — see
  [the Ruby node chapter](../nodes/ruby.md).

## See also

- Runnable example: **`just example walk_ast`**
  (`crates/aozora/examples/walk_ast.rs`) shows the full node walk this
  recipe narrows.
- [Walk the AST](walk-ast.md) — the general traversal.
- [Ruby node reference](../nodes/ruby.md) — the `Ruby` struct, the two
  source forms, and the rendered HTML.
- [Ruby notation](../notation/ruby.md) — the `｜青梅《おうめ》`
  syntax itself.
