//! Verify that canonical source serialization is a fixed point.

#[expect(
    clippy::expect_used,
    clippy::print_stdout,
    reason = "fixed example inputs must parse successfully and the example demonstrates stdout output"
)]
fn main() {
    let source = "青梅《おうめ》街道を行く。";

    let first = aozora::parse(source)
        .expect("source fits parser span limit")
        .snapshot()
        .to_source();

    let second = aozora::parse(first.clone())
        .expect("source fits parser span limit")
        .snapshot()
        .to_source();

    println!("source : {source}");
    println!("first  : {first}");
    println!("second : {second}");

    assert_eq!(
        first, second,
        "serialize must be a fixed point: serialize(parse(serialize(parse(s)))) == serialize(parse(s))"
    );

    assert_eq!(first, source, "this input was already canonical");

    println!("OK: round-trip is a fixed point");
}
