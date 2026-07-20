//! Pull every ruby span out of a document as `(base, reading)` pairs.
//!
//! Plain base and reading text are present directly on the stable ruby
//! projection. Nested notation is reported as `None`.
//!
//! Run with:
//!
//! ```text
//! cargo run --example extract_ruby
//! ```

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
