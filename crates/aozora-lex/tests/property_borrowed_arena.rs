//! Borrowed arena lex equivalence: `aozora_lex::lex_into_arena` ≡
//! `aozora_lex::lex` modulo storage shape.
//!
//! Plan B.2 introduces an arena-emitting lex API that produces a
//! borrowed-AST registry and arena-allocated normalized text. The
//! conversion is value-preserving — a property the owned pipeline's
//! `byte_identical` proptest already pins. This file extends that
//! pinning to the new arena-emitting API by walking both outputs
//! side-by-side and asserting that every observable field matches.
//!
//! Pinned dimensions (mirrors `property_byte_identical.rs`):
//!
//! 1. Normalised text byte-for-byte.
//! 2. `sanitized_len`.
//! 3. Diagnostic count.
//! 4. Inline / block-leaf / block-open / block-close registry sizes.
//!
//! Plus per-entry node-variant equivalence (`xml_node_name` +
//! `is_block`) across the inline and block-leaf tables — confirms
//! the converter routes every variant correctly.

use aozora_syntax::borrowed::Arena;
use aozora_test_utils::config::default_config;
use aozora_test_utils::generators::*;
use proptest::prelude::*;

#[allow(
    clippy::too_many_lines,
    reason = "single assertion stack — splitting would scatter the equivalence checks"
)]
fn assert_arena_equivalent(source: &str) {
    let arena = Arena::new();
    let owned = aozora_lex::lex(source);
    let borrowed = aozora_lex::lex_into_arena(source, &arena);

    assert_eq!(
        borrowed.normalized, owned.normalized,
        "normalized text diverged for input {source:?}"
    );
    assert_eq!(
        borrowed.sanitized_len, owned.sanitized_len,
        "sanitized_len diverged for input {source:?}"
    );
    assert_eq!(
        borrowed.diagnostics.len(),
        owned.diagnostics.len(),
        "diagnostic count diverged for input {source:?}"
    );
    assert_eq!(
        borrowed.registry.inline.len(),
        owned.registry.inline.len(),
        "inline registry length diverged for input {source:?}"
    );
    assert_eq!(
        borrowed.registry.block_leaf.len(),
        owned.registry.block_leaf.len(),
        "block_leaf registry length diverged for input {source:?}"
    );
    assert_eq!(
        borrowed.registry.block_open.len(),
        owned.registry.block_open.len(),
        "block_open registry length diverged for input {source:?}"
    );
    assert_eq!(
        borrowed.registry.block_close.len(),
        owned.registry.block_close.len(),
        "block_close registry length diverged for input {source:?}"
    );

    // Inline + block_leaf: per-position node variant equivalence via
    // xml_node_name. Cheap and exhaustive across the AST surface.
    for ((pos_b, node_b), (pos_o, node_o)) in borrowed
        .registry
        .inline
        .iter_sorted()
        .zip(owned.registry.inline.iter())
    {
        assert_eq!(
            *pos_b, *pos_o,
            "inline[{pos_b}] position diverged for input {source:?}"
        );
        assert_eq!(
            node_b.xml_node_name(),
            node_o.xml_node_name(),
            "inline[{pos_b}] node kind diverged for input {source:?}"
        );
    }

    for ((pos_b, node_b), (pos_o, node_o)) in borrowed
        .registry
        .block_leaf
        .iter_sorted()
        .zip(owned.registry.block_leaf.iter())
    {
        assert_eq!(
            *pos_b, *pos_o,
            "block_leaf[{pos_b}] position diverged for input {source:?}"
        );
        assert_eq!(
            node_b.xml_node_name(),
            node_o.xml_node_name(),
            "block_leaf[{pos_b}] node kind diverged for input {source:?}"
        );
    }

    // Container tables: kind equality (ContainerKind is Copy and Eq).
    for ((pos_b, kind_b), (pos_o, kind_o)) in borrowed
        .registry
        .block_open
        .iter_sorted()
        .zip(owned.registry.block_open.iter())
    {
        assert_eq!(
            *pos_b, *pos_o,
            "block_open[{pos_b}] position diverged for input {source:?}"
        );
        assert_eq!(
            kind_b, kind_o,
            "block_open[{pos_b}] kind diverged for input {source:?}"
        );
    }
    for ((pos_b, kind_b), (pos_o, kind_o)) in borrowed
        .registry
        .block_close
        .iter_sorted()
        .zip(owned.registry.block_close.iter())
    {
        assert_eq!(
            *pos_b, *pos_o,
            "block_close[{pos_b}] position diverged for input {source:?}"
        );
        assert_eq!(
            kind_b, kind_o,
            "block_close[{pos_b}] kind diverged for input {source:?}"
        );
    }
}

// ----------------------------------------------------------------------
// Hand-curated regression anchors — same shapes as the owned-pipeline
// equivalence test. Each anchor exercises one variant family.
// ----------------------------------------------------------------------

#[test]
fn empty_input_is_arena_equivalent() {
    assert_arena_equivalent("");
}

#[test]
fn plain_text_is_arena_equivalent() {
    assert_arena_equivalent("Hello, world.");
    assert_arena_equivalent("こんにちは、世界！");
}

#[test]
fn explicit_ruby_is_arena_equivalent() {
    assert_arena_equivalent("｜青梅《おうめ》");
}

#[test]
fn implicit_ruby_is_arena_equivalent() {
    assert_arena_equivalent("青梅《おうめ》");
}

#[test]
fn double_ruby_is_arena_equivalent() {
    assert_arena_equivalent("《《重要》》");
}

#[test]
fn bracket_annotations_are_arena_equivalent() {
    assert_arena_equivalent("text［＃改ページ］more text");
    assert_arena_equivalent("［＃ここから2字下げ］");
    assert_arena_equivalent("［＃ここで字下げ終わり］");
}

#[test]
fn gaiji_marker_is_arena_equivalent() {
    assert_arena_equivalent("※［＃「木＋吶のつくり」、第3水準1-85-54］");
}

#[test]
fn nested_quoted_annotation_is_arena_equivalent() {
    assert_arena_equivalent("text［＃「青空」に傍点］more");
}

#[test]
fn mixed_pageful_is_arena_equivalent() {
    assert_arena_equivalent(
        "明治の頃｜青梅《おうめ》街道沿いに、※［＃「木＋吶のつくり」、第3水準1-85-54］\n\
         なる珍しき木が立つ。［＃ここから2字下げ］その下で人々は語らひ。\n\
         ［＃ここで字下げ終わり］",
    );
}

// ----------------------------------------------------------------------
// Property tests: cover the same generator surface as the owned-pipeline
// equivalence test so any divergence shows up under both gates.
// ----------------------------------------------------------------------

proptest! {
    #![proptest_config(default_config())]

    #[test]
    fn aozora_fragment_is_arena_equivalent(s in aozora_fragment(120)) {
        assert_arena_equivalent(&s);
    }

    #[test]
    fn pathological_aozora_is_arena_equivalent(s in pathological_aozora(120)) {
        assert_arena_equivalent(&s);
    }

    #[test]
    fn unicode_adversarial_is_arena_equivalent(s in unicode_adversarial()) {
        assert_arena_equivalent(&s);
    }
}
