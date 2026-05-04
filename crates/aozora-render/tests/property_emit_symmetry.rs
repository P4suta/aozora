//! Open/close symmetry on the render and serialise paths.
//!
//! `property_serialize_idempotent` proves a *fixed point*: re-parsing
//! and re-serialising converge. That is too weak to catch a bug where
//! the renderer drops one half of a symmetric pair, because the
//! mangled output may itself parse and re-serialise to the same
//! mangled output — a fixed point that is *wrong*.
//!
//! The `emit_gaiji` close-quote regression observed on 2026-04-25
//! (`「`–`」` lost the closing quote on a gaiji description) is the
//! poster child for this class of bug. The properties below are
//! written specifically to surface it:
//!
//! 1. **Source-pair delta preservation** — the difference (open count
//!    minus close count) for each of `《`/`》`, `「`/`」`, `［＃`/`］`,
//!    and `〔`/`〕` must be identical between the input source and the
//!    serialised output. A renderer that drops a `」` shifts the delta
//!    by one and is detected immediately.
//!
//! 2. **HTML tag balance** — each rendered HTML tag pair (`<p>`,
//!    `<ruby>`, `<rt>`, `<rp>`) must occur the same number of times
//!    open and close. The render-path equivalent of (1).
//!
//! Both properties run over the workhorse generators so any regression
//! that introduces an asymmetric emit will fail under shrinking with a
//! minimal repro pointing at the offending construct.

use aozora_pipeline::lex_into_arena;
use aozora_proptest::config::default_config;
use aozora_proptest::generators::*;
use aozora_render::html::render_to_string;
use aozora_render::serialize::serialize;
use aozora_syntax::borrowed::Arena;
use proptest::prelude::*;

fn count(haystack: &str, needle: &str) -> i64 {
    i64::try_from(haystack.matches(needle).count())
        .expect("substring match count fits in i64 for any conceivable test input")
}

/// Open count minus close count for a single symmetric pair.
fn pair_delta(s: &str, open: &str, close: &str) -> i64 {
    count(s, open) - count(s, close)
}

/// Assert that every tracked source-level pair has the same
/// open-minus-close delta in `output` as in `input`.
///
/// The pairs tracked are the ones whose asymmetry would be a *content*
/// bug rather than a parsing accident: ruby brackets, gaiji description
/// brackets, annotation brackets, and accent brackets.
fn assert_source_pair_deltas_preserved(input: &str, output: &str) {
    for (open, close) in [("《", "》"), ("「", "」"), ("［＃", "］"), ("〔", "〕")] {
        let i = pair_delta(input, open, close);
        let o = pair_delta(output, open, close);
        assert_eq!(
            i, o,
            "pair {open}/{close} delta drifted on serialise: \
             input={i} output={o}\nsource: {input:?}\nserialised: {output:?}"
        );
    }
}

/// Assert that the rendered HTML keeps every tag pair balanced.
///
/// `<p>`, `<ruby>`, `<rt>` and `<rp>` are the tags whose closing form
/// is unambiguous (no attribute variants of the close tag). A renderer
/// regression that drops a closing tag corrupts every downstream HTML
/// consumer; this is the decisive check for that class of bug.
fn assert_html_tag_pairs_balanced(input: &str, html: &str) {
    for tag in ["p", "ruby", "rt", "rp"] {
        let open = count(html, &format!("<{tag}>"));
        let close = count(html, &format!("</{tag}>"));
        assert_eq!(
            open, close,
            "<{tag}> tag pair unbalanced: open={open} close={close}\nsource: {input:?}\nhtml: {html}"
        );
    }
}

fn assert_emit_symmetry(source: &str) {
    let arena = Arena::new();
    let out = lex_into_arena(source, &arena);
    let serialised = serialize(&out);
    assert_source_pair_deltas_preserved(source, &serialised);

    let html = render_to_string(&out);
    assert_html_tag_pairs_balanced(source, &html);
}

// ----------------------------------------------------------------------
// Hand-curated regression anchors.
// ----------------------------------------------------------------------

#[test]
fn empty_input_is_symmetric() {
    assert_emit_symmetry("");
}

#[test]
fn plain_text_is_symmetric() {
    assert_emit_symmetry("Hello, world.");
    assert_emit_symmetry("こんにちは。\n\n本日は晴れ。");
}

#[test]
fn ruby_is_symmetric() {
    assert_emit_symmetry("｜青梅《おうめ》");
    assert_emit_symmetry("青梅《おうめ》");
}

#[test]
fn paired_container_is_symmetric() {
    assert_emit_symmetry(
        "［＃ここから2字下げ］\n\
         body\n\
         ［＃ここで字下げ終わり］",
    );
}

/// Direct anchor for the 2026-04-25 `emit_gaiji` close-quote regression.
/// The gaiji description carries a `「`/`」` pair that the serialiser
/// must round-trip with both halves intact.
#[test]
fn gaiji_with_close_quote_is_symmetric() {
    assert_emit_symmetry("※［＃「あ」、第1水準1-1］");
    assert_emit_symmetry("text ※［＃「複数の文字」、第3水準1-15-23］ tail");
}

proptest! {
    #![proptest_config(default_config())]

    /// Workhorse generator — every Aozora-shaped fragment must round-trip
    /// with every tracked source pair preserved and every HTML tag
    /// pair balanced.
    #[test]
    fn aozora_fragment_emit_is_symmetric(s in aozora_fragment(120)) {
        assert_emit_symmetry(&s);
    }

    /// Pathological / unbalanced shapes — even when the *input* is
    /// already asymmetric, the serialiser must preserve the same
    /// delta (i.e. it neither adds nor removes a half-pair).
    #[test]
    fn pathological_input_emit_is_symmetric(s in pathological_aozora(120)) {
        assert_emit_symmetry(&s);
    }

    /// Deeply nested same-shape pairs — a regression that miscounts
    /// stack frames would shift a delta and is caught here under
    /// shrinking.
    #[test]
    fn nested_pairs_emit_is_symmetric(s in nested_pairs(64)) {
        assert_emit_symmetry(&s);
    }

    /// Unicode adversarial — combining marks, RTL overrides, PUA,
    /// full-width forms. The renderer must remain symmetric across
    /// every Unicode shape.
    #[test]
    fn unicode_adversarial_emit_is_symmetric(s in unicode_adversarial()) {
        assert_emit_symmetry(&s);
    }
}
