//! Print classified node kinds and their original source text.

#[expect(
    clippy::expect_used,
    clippy::print_stdout,
    reason = "fixed example inputs must parse successfully and the example demonstrates stdout output"
)]
fn main() {
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
