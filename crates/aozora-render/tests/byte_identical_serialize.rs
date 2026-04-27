//! Pin the Aozora-source serializer's "fixed point after one pass"
//! invariant.
//!
//! Contract: for every input that the lexer can ingest, running
//! `serialize ∘ parse` once produces a string `s`. A second pass —
//! `serialize ∘ parse(s)` — must produce the same `s` byte-for-byte.
//! This is workspace invariant I3 (see `serialize.rs` doc comment +
//! ADR-0005).

use aozora_lex::lex_into_arena;
use aozora_render::serialize::serialize;
use aozora_syntax::borrowed::Arena;
use proptest::prelude::*;

fn round_trip(src: &str) -> String {
    let arena = Arena::new();
    let out = lex_into_arena(src, &arena);
    serialize(&out)
}

fn fixed_point(src: &str) -> bool {
    let one = round_trip(src);
    let two = round_trip(&one);
    one == two
}

#[test]
fn fixed_point_on_empty_input() {
    assert!(fixed_point(""));
}

#[test]
fn fixed_point_on_pure_ascii() {
    assert!(fixed_point("hello, world"));
}

#[test]
fn fixed_point_on_pure_japanese_prose() {
    assert!(fixed_point("青空文庫の本文。"));
}

#[test]
fn fixed_point_on_explicit_ruby() {
    assert!(fixed_point("｜青梅《おうめ》"));
}

#[test]
fn fixed_point_on_inline_ruby() {
    assert!(fixed_point("青梅《おうめ》"));
}

#[test]
fn fixed_point_on_page_break() {
    assert!(fixed_point("前\n\n［＃改ページ］\n\n後"));
}

#[test]
fn fixed_point_on_paragraph_breaks() {
    assert!(fixed_point("a\n\nb\n\nc"));
}

#[test]
fn fixed_point_on_html_unsafe_chars_in_source() {
    // Source contains literal `<`, `>`, `&`, `"`, `'`. Serialization
    // must NOT escape them — only HTML rendering does.
    let src = "a<b>&\"'";
    let out = round_trip(src);
    assert_eq!(out, src, "serializer must not escape HTML metachars");
    assert!(fixed_point(src));
}

#[test]
fn fixed_point_on_gaiji_reference() {
    // Real Aozora source: 「※［＃「木＋吶のつくり」、第3水準1-85-54］」
    let src = "※［＃「木＋吶のつくり」、第3水準1-85-54］";
    assert!(fixed_point(src));
}

#[test]
fn fixed_point_on_kaeriten() {
    let src = "学［＃二、レ点］而時習之";
    assert!(fixed_point(src));
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        .. ProptestConfig::default()
    })]

    /// Plain mixed text — letters, digits, hiragana, kanji, newlines,
    /// HTML-unsafe ASCII — must round-trip to a fixed point. The
    /// regex avoids generating Aozora markup characters that the
    /// lexer would split on; that subset is exercised in the
    /// hand-written tests above (and in the lex-pipeline crate).
    #[test]
    fn arbitrary_plain_text_is_a_fixed_point(
        s in "[A-Za-z0-9 \n<>&\"'\u{3042}-\u{3093}\u{4E00}-\u{4E20}]{0,80}",
    ) {
        prop_assert!(fixed_point(&s), "non-fixed point on input {:?}", s);
    }

    /// Length monotonicity: serialization can grow the input (PUA
    /// sentinels expand back to multi-char Aozora markers) but for
    /// pure-text input with no markup it must produce the same
    /// length.
    #[test]
    fn pure_text_length_preserved(
        s in "[A-Za-z0-9 \u{3042}-\u{3093}]{0,80}",
    ) {
        let out = round_trip(&s);
        prop_assert_eq!(
            out.len(), s.len(),
            "pure text input grew on serialize: {:?} -> {:?}", s, out,
        );
    }
}
