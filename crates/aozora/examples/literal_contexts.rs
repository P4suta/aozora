//! Recover the exact bytes an author typed for each notation.
//!
//! `source_span` records where a node came from, so slicing the original
//! buffer gives the text back verbatim — before any canonicalisation.
//! A host that must show a notation literally (inside a code span, say)
//! emits this rather than the resolved form.
//!
//! Run with:
//!
//! ```text
//! cargo run --example literal_contexts
//! ```

fn main() {
    let source = "冒頭｜青梅《おうめ》。［＃改ページ］次の章。";
    let doc = aozora::parse(source).expect("source fits parser span limit");
    let tree = doc.snapshot();

    for node in tree.nodes() {
        println!("{}", tree.slice(node.span()).expect("node span is valid"));
    }
}
