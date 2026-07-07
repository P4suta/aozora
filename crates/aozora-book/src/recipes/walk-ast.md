# Walk the AST

**Problem.** You have parsed a document and want to visit every
classified Aozora construct in source order — to count node kinds,
build an index, or drive a custom renderer.

## Solution

`Tree::source_nodes` returns a slice of `SourceNode`, one per
classified construct, sorted by source position. Each carries a
`source_span` (byte offsets into the source) and a `node`, which is a
`NodeRef` tagging the sentinel kind that fired.

```rust
# extern crate aozora;
use aozora::{Document, NodeRef};

fn main() {
    let source = "｜青梅《おうめ》の［＃ここから2字下げ］街道《かいどう》［＃ここで字下げ終わり］";
    let doc = Document::new(source);
    let tree = doc.parse();

    for sn in tree.source_nodes() {
        let span = sn.source_span;
        match sn.node {
            NodeRef::Inline(node) | NodeRef::BlockLeaf(node) => {
                // `node` is a Node; `.kind()` is the cross-cutting tag.
                println!("{:>3}..{:<3} {:?}", span.start, span.end, node.kind());
            }
            NodeRef::BlockOpen(kind) => {
                println!("{:>3}..{:<3} open  {kind:?}", span.start, span.end);
            }
            NodeRef::BlockClose(kind) => {
                println!("{:>3}..{:<3} close {kind:?}", span.start, span.end);
            }
            // `NodeRef` is `#[non_exhaustive]`: a wildcard keeps the
            // match valid as future sentinel kinds are added.
            _ => {}
        }
    }
}
```

## Expected output

```text
  0..21  Ruby
 24..45  open  Indent { amount: 2 }
 45..72  Ruby
 72..105 close Indent { amount: 2 }
```

(Byte offsets are over the full-width UTF-8 source; the exact numbers
depend on your input.)

## How the surface is shaped

`source_nodes()` is the source-coordinate view — the one editor
features and indexers want. The `NodeRef` variant tells you where
the construct landed:

- `Inline` — an inline construct (ruby, bouten, gaiji, 縦中横, …)
  carrying a `Node`.
- `BlockLeaf` — a standalone block construct (page break, section
  break, heading) carrying a `Node`.
- `BlockOpen` / `BlockClose` — the two ends of a paired container
  (`［＃ここから…］` / `［＃ここで…終わり］`), carrying a `RegionFormat`
  (open) and a `RegionClose` (close) respectively.

`NodeRef::kind()` collapses all four into a single
[`NodeKind`][nodekind] tag when you only need the discriminant.

### Matching container open/close pairs

The walk above sees opens and closes as independent events. When you
need them *paired* — "where does this `［＃ここから…］` close?" —
read `Tree::container_pairs` instead, which yields one entry per
balanced pair (in normalized coordinates). The inline-delimiter
analogue (ruby `《…》`, brackets) is `Tree::pairs`. See
[Indent & align containers](../notation/indent.md) for the container
model.

### Reaching inside a node

`Node` is an owned, lifetime-free enum; its payload fields hold
the construct's content as `StrId` / `ContentRange` handles into the
tree's `NodeStore`. To pull text out of a specific variant — say the
base and reading of a ruby node — match the variant and resolve its
content against the store; that is the next recipe,
[Extract ruby pairs](extract-ruby.md).

[nodekind]: https://docs.rs/aozora/latest/aozora/enum.NodeKind.html

## See also

- Runnable example: **`just example walk_ast`**
  (`crates/aozora/examples/walk_ast.rs`).
- [Extract ruby pairs](extract-ruby.md) — the same walk, narrowed to
  one node kind, reading its content.
- [Library Quickstart → Walking the AST](../getting-started/library.md#walking-the-ast).
- [Node reference](../nodes/index.md) — every `NodeKind` and what it
  carries.
- [Owned AST & NodeStore](../arch/arena.md) — how the owned AST relates
  to the `Document`'s source lifetime.
