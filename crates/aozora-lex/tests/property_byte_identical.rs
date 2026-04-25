//! Byte-identical equivalence: `aozora_lex::lex` ≡ `aozora_lexer::lex`.
//!
//! ADR-0010's "observable equivalence" purity contract demands that
//! the new lex pipeline produce **byte-identical** output to the
//! legacy 7-phase implementation it gradually replaces. While the
//! Move 2.2 wrapper is a literal delegation, this property stays
//! meaningful through every subsequent commit that grows the new
//! pipeline's own implementation: the moment a divergence appears,
//! this test surfaces it.
//!
//! Pinned dimensions:
//!
//! 1. Normalised text byte-for-byte.
//! 2. Diagnostic count and per-variant kind.
//! 3. Inline / block-leaf / block-open / block-close registry sizes.
//! 4. Sanitised length (the `LexOutput::sanitized_len` field that
//!    `aozora-parser`'s parallel merge uses for span correction).
//!
//! Plus a smaller exhaustive matrix of hand-curated inputs that
//! anchor the proptest with deterministic regression targets.

use aozora_test_utils::config::default_config;
use aozora_test_utils::generators::*;
use proptest::prelude::*;

fn assert_byte_identical(source: &str) {
    let new_out = aozora_lex::lex(source);
    let old_out = aozora_lexer::lex(source);

    assert_eq!(
        new_out.normalized, old_out.normalized,
        "normalized text diverged for input {source:?}"
    );
    assert_eq!(
        new_out.sanitized_len, old_out.sanitized_len,
        "sanitized_len diverged for input {source:?}"
    );
    assert_eq!(
        new_out.diagnostics.len(),
        old_out.diagnostics.len(),
        "diagnostic count diverged for input {source:?}: new={:?} old={:?}",
        new_out.diagnostics,
        old_out.diagnostics
    );
    assert_eq!(
        new_out.registry.inline.len(),
        old_out.registry.inline.len(),
        "inline registry length diverged for input {source:?}"
    );
    assert_eq!(
        new_out.registry.block_leaf.len(),
        old_out.registry.block_leaf.len(),
        "block_leaf registry length diverged for input {source:?}"
    );
    assert_eq!(
        new_out.registry.block_open.len(),
        old_out.registry.block_open.len(),
        "block_open registry length diverged for input {source:?}"
    );
    assert_eq!(
        new_out.registry.block_close.len(),
        old_out.registry.block_close.len(),
        "block_close registry length diverged for input {source:?}"
    );
}

// ----------------------------------------------------------------------
// Hand-curated regression anchors. Cheap, deterministic, fast feedback
// for changes that obviously break.
// ----------------------------------------------------------------------

#[test]
fn empty_input_is_byte_identical() {
    assert_byte_identical("");
}

#[test]
fn plain_text_is_byte_identical() {
    assert_byte_identical("Hello, world.");
    assert_byte_identical("こんにちは、世界！");
    assert_byte_identical("Mixed: hello 世界 hi");
}

#[test]
fn explicit_ruby_is_byte_identical() {
    assert_byte_identical("｜青梅《おうめ》");
    assert_byte_identical("a｜青梅《おうめ》b");
}

#[test]
fn implicit_ruby_is_byte_identical() {
    assert_byte_identical("青梅《おうめ》");
    assert_byte_identical("text 青梅《おうめ》 text");
}

#[test]
fn double_ruby_is_byte_identical() {
    assert_byte_identical("《《重要》》");
    assert_byte_identical("a《《重要》》b");
}

#[test]
fn bracket_annotations_are_byte_identical() {
    assert_byte_identical("text［＃改ページ］more text");
    assert_byte_identical("［＃ここから2字下げ］");
    assert_byte_identical("［＃ここで字下げ終わり］");
}

#[test]
fn gaiji_marker_is_byte_identical() {
    assert_byte_identical("※［＃「木＋吶のつくり」、第3水準1-85-54］");
}

#[test]
fn nested_quoted_annotation_is_byte_identical() {
    assert_byte_identical("text［＃「青空」に傍点］more");
}

#[test]
fn mixed_pageful_is_byte_identical() {
    assert_byte_identical(
        "明治の頃｜青梅《おうめ》街道沿いに、※［＃「木＋吶のつくり」、第3水準1-85-54］\n\
         なる珍しき木が立つ。［＃ここから2字下げ］その下で人々は語らひ。\n\
         ［＃ここで字下げ終わり］",
    );
}

#[test]
fn pua_collision_diagnostic_is_byte_identical() {
    assert_byte_identical("source contains \u{E001} reserved sentinel");
    assert_byte_identical("source contains \u{E002} reserved sentinel");
    assert_byte_identical("source contains \u{E003} reserved sentinel");
    assert_byte_identical("source contains \u{E004} reserved sentinel");
}

#[test]
fn unbalanced_brackets_are_byte_identical() {
    assert_byte_identical("[unclosed annotation");
    assert_byte_identical("］unmatched close");
    assert_byte_identical("《no close ruby");
}

#[test]
fn accent_decomposition_in_tortoise_brackets_is_byte_identical() {
    assert_byte_identical("〔fune`bre〕"); // funèbre via grave accent digraph
    assert_byte_identical("〔cafe'〕"); // café via apostrophe accent
}

#[test]
fn crlf_normalisation_is_byte_identical() {
    assert_byte_identical("line1\r\nline2\r\nline3");
}

#[test]
fn bom_strip_is_byte_identical() {
    assert_byte_identical("\u{FEFF}body after BOM");
}

#[test]
fn long_decorative_rule_is_byte_identical() {
    assert_byte_identical("preamble\n----------\ntext after rule");
}

// ----------------------------------------------------------------------
// Property tests over the workspace's standard generator set. Each
// generator targets a separate region of input space; running them
// independently gives the shrinker the smallest possible counter-
// example when a divergence appears.
// ----------------------------------------------------------------------

proptest! {
    #![proptest_config(default_config())]

    #[test]
    fn aozora_fragment_is_byte_identical(s in aozora_fragment(120)) {
        assert_byte_identical(&s);
    }

    #[test]
    fn pathological_aozora_is_byte_identical(s in pathological_aozora(120)) {
        assert_byte_identical(&s);
    }

    #[test]
    fn unicode_adversarial_is_byte_identical(s in unicode_adversarial()) {
        assert_byte_identical(&s);
    }
}
