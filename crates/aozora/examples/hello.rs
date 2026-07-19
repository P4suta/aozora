//! Smallest possible aozora program: parse one ruby span, then print
//! both the rendered HTML and the byte-exact serialized source.
//!
//! Run with:
//!
//! ```text
//! cargo run --example hello
//! ```

use aozora::Document;

fn main() {
    // `｜青梅《おうめ》` is explicit ruby: base text 青梅, reading おうめ.
    // `Document` owns the source buffer; the returned `Tree` borrows
    // that source and so lives only as long as `doc` (its owned AST
    // data carries no arena lifetime).
    let doc = Document::new("｜青梅《おうめ》");
    let tree = doc.snapshot();

    // Semantic HTML5 — a <ruby> element with the reading in <rt>.
    println!("--- to_html ---");
    println!("{}", tree.to_html());

    // Byte-exact re-emission of the canonical Aozora source.
    println!("--- serialize ---");
    println!("{}", tree.to_source());
}
