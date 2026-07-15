//! Pull every ruby span out of a document as `(base, reading)` pairs.
//!
//! Shows how to resolve a node's content: `base` and `reading` are
//! `ContentRange` handles into the parse's store, not strings, so they
//! are looked up rather than read. `content_range_as_plain` covers the
//! common case where the content is plain text, and returns `None` when
//! it holds nested constructs.
//!
//! Run with:
//!
//! ```text
//! cargo run --example extract_ruby
//! ```

use aozora::{Document, Node, NodeRef};

fn main() {
    let source = "｜青梅《おうめ》街道を｜逢《お》う";
    let doc = Document::new(source);
    let tree = doc.parse();
    let store = &tree.lex_output().store;

    for sn in tree.source_nodes() {
        // Ruby is always inline.
        if let NodeRef::Inline(Node::Ruby(ruby)) = sn.node {
            let base = store.content_range_as_plain(ruby.base).unwrap_or("<mixed>");
            let reading = store
                .content_range_as_plain(ruby.reading)
                .unwrap_or("<mixed>");
            println!("{base}\t{reading}");
        }
    }
}
