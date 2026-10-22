//! Print ruby bases and readings.

#[expect(
    clippy::expect_used,
    clippy::print_stdout,
    reason = "fixed example inputs must parse successfully and the example demonstrates stdout output"
)]
fn main() {
    let source = "｜青梅《おうめ》街道を｜逢《お》う";
    let doc = aozora::parse(source).expect("source fits parser span limit");
    let snapshot = doc.snapshot();
    for ruby in snapshot.rubies() {
        let base = ruby.base().unwrap_or("<mixed>");
        let reading = ruby.reading().unwrap_or("<mixed>");
        println!("{base}\t{reading}");
    }
}
