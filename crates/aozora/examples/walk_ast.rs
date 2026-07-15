//! Walk the classified source nodes of a parse and print each node's
//! kind alongside the source bytes it covers. `source_nodes()` is the
//! source-ordered side table editor surfaces use for semantic tokens
//! and document symbols.
//!
//! Needs nothing beyond the umbrella crate.
//!
//! Run with:
//!
//! ```text
//! cargo run --example walk_ast
//! ```

use aozora::Document;

fn main() {
    // One ruby span (｜青梅《おうめ》) and one bouten span
    // (［＃「青空」に傍点］), separated by plain text.
    let source = "｜青梅《おうめ》の下、［＃「青空」に傍点］を見る。";
    let doc = Document::new(source);
    let tree = doc.parse();

    // `source_nodes()` yields one `SourceNode` per classified Aozora /
    // container span, sorted by `source_span.start`. Plain-text runs
    // between constructs are not listed — they round-trip verbatim.
    println!("{} classified node(s)", tree.source_nodes().len());

    for entry in tree.source_nodes() {
        let span = entry.source_span;
        // `entry.node` is a `NodeRef`: Inline / BlockLeaf / BlockOpen /
        // BlockClose, each wrapping the owned AST node or container
        // kind. Its Debug form is a compact, accurate kind label.
        println!(
            "{:>3}..{:<3}  {:<10}  node={:?}",
            span.start,
            span.end,
            format!("{:?}", span.slice(source)),
            entry.node,
        );
    }
}
