//! `serialize` is a fixed point: re-parsing serialized output and
//! serializing again yields byte-identical text. This is the property
//! `aozora fmt --check` exercises in CI and the corpus sweep verifies
//! across the whole Aozora Bunko catalogue.
//!
//! the `aozora` umbrella surface.
//!
//! Run with:
//!
//! ```text
//! cargo run --example round_trip
//! ```

fn main() {
    // A canonical Aozora source: bare ruby (the all-kanji base needs no
    // explicit `｜`) plus trailing plain text.
    let source = "青梅《おうめ》街道を行く。";

    // First pass: parse the original, serialize back to Aozora source.
    let first = aozora::parse(source)
        .expect("source fits parser span limit")
        .snapshot()
        .to_source();

    // Second pass: parse THAT output and serialize again.
    let second = aozora::parse(first.clone())
        .expect("source fits parser span limit")
        .snapshot()
        .to_source();

    println!("source : {source}");
    println!("first  : {first}");
    println!("second : {second}");

    // The canonical form is stable: once serialized, further
    // parse/serialize cycles change nothing.
    assert_eq!(
        first, second,
        "serialize must be a fixed point: serialize(parse(serialize(parse(s)))) == serialize(parse(s))"
    );

    // For input that is already canonical, the very first serialize
    // also equals the source.
    assert_eq!(first, source, "this input was already canonical");

    println!("OK: round-trip is a fixed point");
}
