//! Walk the classified source nodes of a parse and print each node's
//! kind alongside the source bytes it covers. `nodes()` is the
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

fn main() {
    // One ruby span (｜青梅《おうめ》) and one bouten span
    // (［＃「青空」に傍点］), separated by plain text.
    let source = "｜青梅《おうめ》の下、［＃「青空」に傍点］を見る。";
    let doc = aozora::parse(source).expect("source fits parser span limit");
    let tree = doc.snapshot();

    println!("{} classified node(s)", tree.nodes().len());

    for node in tree.nodes() {
        let span = node.span();
        println!(
            "{:>3}..{:<3}  {:<10}  kind={:?}",
            span.start,
            span.end,
            format!("{:?}", tree.slice(span).expect("node span is valid")),
            node.kind(),
        );
    }
}
