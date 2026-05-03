//! Serialisation idempotence: `serialize ∘ parse ∘ serialize ∘ parse`
//! must equal `serialize ∘ parse`.
//!
//! The serialise path round-trips a parsed tree back to Aozora source
//! text. The natural "parse(serialize(parse(s))) == parse(s)" statement
//! requires AST equality; here we use the weaker but easier-to-verify
//! property that re-parsing and re-serialising must converge to a
//! fixed point. If the serialiser ever introduces non-canonical
//! whitespace or character variants that the parser strips, the
//! second pass would *change* the bytes, and the property fails.

use aozora_pipeline::lex_into_arena;
use aozora_proptest::config::default_config;
use aozora_proptest::generators::*;
use aozora_render::serialize::serialize;
use aozora_syntax::borrowed::Arena;
use proptest::prelude::*;

fn round_trip_once(source: &str) -> String {
    let arena = Arena::new();
    let out = lex_into_arena(source, &arena);
    serialize(&out)
}

fn assert_serialise_is_idempotent(source: &str) {
    let first = round_trip_once(source);
    let second = round_trip_once(&first);
    assert_eq!(
        first, second,
        "serialise round-trip diverges on second pass for source {source:?}\n\
         after 1st: {first:?}\n\
         after 2nd: {second:?}"
    );
}

// ----------------------------------------------------------------------
// Hand-curated regression anchors.
// ----------------------------------------------------------------------

#[test]
fn empty_input_is_idempotent() {
    assert_serialise_is_idempotent("");
}

#[test]
fn plain_text_is_idempotent() {
    assert_serialise_is_idempotent("Hello, world.");
    assert_serialise_is_idempotent("こんにちは。\n\n本日は晴れ。");
}

#[test]
fn ruby_is_idempotent() {
    assert_serialise_is_idempotent("｜青梅《おうめ》");
    assert_serialise_is_idempotent("青梅《おうめ》");
}

#[test]
fn paired_container_is_idempotent() {
    assert_serialise_is_idempotent(
        "［＃ここから2字下げ］\n\
         body\n\
         ［＃ここで字下げ終わり］",
    );
}

proptest! {
    #![proptest_config(default_config())]

    /// `serialize ∘ parse ∘ serialize ∘ parse = serialize ∘ parse` over
    /// the workhorse generator. A divergence between passes means the
    /// serialise output is *not* a parser fixed point — a quiet bug
    /// that would silently mutate documents on every round-trip.
    #[test]
    fn aozora_fragment_is_idempotent(s in aozora_fragment(120)) {
        assert_serialise_is_idempotent(&s);
    }

    #[test]
    fn pathological_input_is_idempotent(s in pathological_aozora(120)) {
        assert_serialise_is_idempotent(&s);
    }

    #[test]
    fn unicode_adversarial_is_idempotent(s in unicode_adversarial()) {
        assert_serialise_is_idempotent(&s);
    }
}
