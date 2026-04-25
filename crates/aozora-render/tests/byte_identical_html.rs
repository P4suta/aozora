//! Byte-identical equivalence: borrowed-AST HTML render ≡ owned-AST.
//!
//! Plan B.3's borrowed-AST renderer must produce the same bytes as
//! the legacy `aozora_parser::html::render_to_string` for every
//! input. Rendering is the user-facing surface; any divergence here
//! breaks downstream consumers (book theme, indexer, screen-reader
//! integrations) that key on specific class tokens or structural
//! whitespace.
//!
//! This file pins the equivalence with:
//!
//! 1. Hand-curated regression anchors covering each major Aozora
//!    construct (ruby / bouten / gaiji / page break / container /
//!    forward bouten / mixed dense paragraph).
//! 2. Property tests over the workspace generator surface
//!    (`aozora_fragment`, `pathological_aozora`, `unicode_adversarial`)
//!    so any future renderer divergence shows up under shrinking.

use aozora_render::{html, legacy};
use aozora_syntax::borrowed::Arena;
use aozora_test_utils::config::default_config;
use aozora_test_utils::generators::*;
use proptest::prelude::*;

fn assert_html_equal(source: &str) {
    let arena = Arena::new();
    let borrowed_lex = aozora_lex::lex_into_arena(source, &arena);
    let borrowed = html::render_to_string(&borrowed_lex);
    let owned = legacy::html::render_to_string(source);
    assert_eq!(
        borrowed, owned,
        "borrowed HTML diverged for input {source:?}"
    );
}

#[test]
fn empty_input_renders_identically() {
    assert_html_equal("");
}

#[test]
fn plain_text_renders_identically() {
    assert_html_equal("Hello, world.");
    assert_html_equal("こんにちは、世界！");
}

#[test]
fn explicit_ruby_renders_identically() {
    assert_html_equal("｜青梅《おうめ》");
    assert_html_equal("a｜青梅《おうめ》b");
}

#[test]
fn implicit_ruby_renders_identically() {
    assert_html_equal("text 青梅《おうめ》 text");
}

#[test]
fn double_ruby_renders_identically() {
    assert_html_equal("a《《重要》》b");
}

#[test]
fn page_break_renders_identically() {
    assert_html_equal("text［＃改ページ］more text");
}

#[test]
fn container_indent_renders_identically() {
    assert_html_equal("［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］");
}

#[test]
fn gaiji_marker_renders_identically() {
    assert_html_equal("※［＃「木＋吶のつくり」、第3水準1-85-54］");
}

#[test]
fn forward_bouten_renders_identically() {
    assert_html_equal("text［＃「青空」に傍点］more");
}

#[test]
fn html_unsafe_chars_render_identically() {
    assert_html_equal("a<b>&\"'");
    assert_html_equal("text with <html> & \"quotes\" plus 'apostrophe'");
}

#[test]
fn newlines_render_identically() {
    assert_html_equal("a\nb");
    assert_html_equal("a\n\nb");
    assert_html_equal("a\n\n\nb");
    assert_html_equal("");
}

#[test]
fn mixed_dense_paragraph_renders_identically() {
    assert_html_equal(
        "明治の頃｜青梅《おうめ》街道沿いに、※［＃「木＋吶のつくり」、第3水準1-85-54］\n\
         なる珍しき木が立つ。［＃ここから2字下げ］その下で人々は語らひ。\n\
         ［＃ここで字下げ終わり］",
    );
}

#[test]
fn pua_collision_diagnostic_inputs_render_identically() {
    assert_html_equal("source contains \u{E001} reserved sentinel");
}

proptest! {
    #![proptest_config(default_config())]

    #[test]
    fn aozora_fragment_renders_identically(s in aozora_fragment(120)) {
        assert_html_equal(&s);
    }

    #[test]
    fn pathological_aozora_renders_identically(s in pathological_aozora(120)) {
        assert_html_equal(&s);
    }

    #[test]
    fn unicode_adversarial_renders_identically(s in unicode_adversarial()) {
        assert_html_equal(&s);
    }
}
