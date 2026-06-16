//! `serialize` is a fixed point: re-parsing serialized output and
//! serializing again yields byte-identical text. This is the property
//! `aozora fmt --check` exercises in CI and the corpus sweep verifies
//! across the whole Aozora Bunko catalogue.
//!
//! Handbook recipe: <https://p4suta.github.io/aozora/getting-started/library.html>
//! (Library Quickstart → Round-trip and canonicalisation). Uses only
//! the `aozora` umbrella surface.
//!
//! Run with:
//!
//! ```text
//! cargo run --example round_trip
//! ```

use aozora::Document;

fn main() {
    // A canonical Aozora source: explicit ruby plus trailing plain text.
    let source = "｜青梅《おうめ》街道を行く。";

    // First pass: parse the original, serialize back to Aozora source.
    let first = Document::new(source).parse().serialize();

    // Second pass: parse THAT output and serialize again.
    let second = Document::new(first.clone()).parse().serialize();

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
