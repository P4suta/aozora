//! Public-API totality + idempotence for [`Document::parse`].
//!
//! Two complementary properties on the top-facade entry point:
//!
//! 1. **Totality**: [`Document::parse`] must not panic on any input.
//!    The lex pipeline emits diagnostics rather than failing, and the
//!    facade is the sole entry point for FFI / WASM / Python drivers
//!    that hand user input straight to the parser. A panic here is a
//!    denial-of-service surface for every downstream binding.
//!
//! 2. **Parse → serialise → parse converges**: the second
//!    `Document::parse` over the serialised output of the first
//!    yields a tree whose own `serialize()` matches. Equivalent to
//!    "the renderer's `serialize` is a parser fixed point" — a
//!    quieter way to surface bugs that mutate documents on round-trip
//!    (whitespace drift, character substitutions). Functionally
//!    overlaps the `aozora-render` `property_serialize_idempotent`
//!    test, but exercised here through the **public API only** to
//!    catch facade regressions (e.g. a `Document` that builds an
//!    inconsistent arena).

use aozora::Document;
use aozora_proptest::config::default_config;
use aozora_proptest::generators::*;
use proptest::prelude::*;

fn parse_serialise_parse(source: &str) -> (String, String) {
    let doc = Document::new(source.to_owned());
    let tree = doc.parse();
    let first = tree.serialize();
    let doc2 = Document::new(first.clone());
    let tree2 = doc2.parse();
    let second = tree2.serialize();
    (first, second)
}

fn assert_facade_round_trip_converges(source: &str) {
    let (first, second) = parse_serialise_parse(source);
    assert_eq!(
        first, second,
        "facade round-trip diverges on second pass for source {source:?}\n\
         after 1st: {first:?}\n\
         after 2nd: {second:?}"
    );
}

// ----------------------------------------------------------------------
// Hand-curated regression anchors.
// ----------------------------------------------------------------------

#[test]
fn empty_input_round_trips() {
    assert_facade_round_trip_converges("");
}

#[test]
fn plain_text_round_trips() {
    assert_facade_round_trip_converges("Hello, world.");
    assert_facade_round_trip_converges("こんにちは。\n\n本日は晴れ。");
}

#[test]
fn ruby_round_trips() {
    assert_facade_round_trip_converges("｜青梅《おうめ》");
    assert_facade_round_trip_converges("青梅《おうめ》");
}

#[test]
fn paired_container_round_trips() {
    assert_facade_round_trip_converges("［＃ここから2字下げ］\nbody\n［＃ここで字下げ終わり］");
}

proptest! {
    #![proptest_config(default_config())]

    /// `Document::parse` is total over the workhorse Aozora fragment
    /// distribution. A panic here is a DoS surface for every public
    /// caller — proptest is the decisive way to catch one.
    #[test]
    fn aozora_fragment_parse_is_total(s in aozora_fragment(120)) {
        let doc = Document::new(s);
        let _tree = doc.parse();
    }

    /// Parse → serialise → parse converges on the second pass. The
    /// serialiser must produce a parser fixed point.
    #[test]
    fn aozora_fragment_round_trip_converges(s in aozora_fragment(120)) {
        assert_facade_round_trip_converges(&s);
    }

    /// Pathological / unbalanced inputs — public parse must stay
    /// total even when diagnostics fire.
    #[test]
    fn pathological_input_parse_is_total(s in pathological_aozora(120)) {
        let doc = Document::new(s);
        let _tree = doc.parse();
    }

    /// Unicode adversarial — combining marks, RTL overrides, PUA
    /// bytes that the lexer reserves for sentinel use. Public parse
    /// must stay total and emit `SourceContainsPua` diagnostics
    /// rather than panicking.
    #[test]
    fn unicode_adversarial_parse_is_total(s in unicode_adversarial()) {
        let doc = Document::new(s);
        let _tree = doc.parse();
    }
}
