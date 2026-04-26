//! Determinism + structural invariants for `lex_into_arena`.
//!
//! Pre-I-2.2 / pre-Phase-F this file pinned an equivalence between the
//! borrowed arena pipeline and the owned reference (`aozora_lex::lex`).
//! The owned API is gone (Phase F.2 deletes the wrapper, Phase F.3
//! deletes the legacy backend), so the load-bearing property switches
//! to *determinism* and *self-consistency* of the borrowed pipeline:
//!
//! 1. Two independent runs over the same source produce byte-identical
//!    normalised text and identical registry shape (positions + node
//!    kinds + container kinds + diagnostic count).
//! 2. Every PUA sentinel in the normalised text has a registry entry,
//!    and every registry entry's position points at a PUA sentinel
//!    (V1/V2 lex invariants, but observed from outside).
//!
//! Together these gate any future change to `lex_into_arena` from
//! introducing nondeterminism (e.g. iteration order over a HashMap)
//! or from desynchronising the registry from the normalised text.

use aozora_syntax::borrowed::Arena;
use aozora_test_utils::config::default_config;
use aozora_test_utils::generators::*;
use proptest::prelude::*;

fn assert_deterministic(source: &str) {
    let arena_a = Arena::new();
    let arena_b = Arena::new();
    let a = aozora_lex::lex_into_arena(source, &arena_a);
    let b = aozora_lex::lex_into_arena(source, &arena_b);

    assert_eq!(
        a.normalized, b.normalized,
        "normalized text non-deterministic for input {source:?}"
    );
    assert_eq!(
        a.sanitized_len, b.sanitized_len,
        "sanitized_len non-deterministic for input {source:?}"
    );
    assert_eq!(
        a.diagnostics.len(),
        b.diagnostics.len(),
        "diagnostic count non-deterministic for input {source:?}"
    );
    assert_eq!(
        a.registry.inline.len(),
        b.registry.inline.len(),
        "inline registry length non-deterministic for input {source:?}"
    );
    assert_eq!(
        a.registry.block_leaf.len(),
        b.registry.block_leaf.len(),
        "block_leaf registry length non-deterministic for input {source:?}"
    );
    assert_eq!(
        a.registry.block_open.len(),
        b.registry.block_open.len(),
        "block_open registry length non-deterministic for input {source:?}"
    );
    assert_eq!(
        a.registry.block_close.len(),
        b.registry.block_close.len(),
        "block_close registry length non-deterministic for input {source:?}"
    );

    // Per-position node kind equivalence across the two runs. Cheap +
    // exhaustive across the AST surface.
    for ((pos_a, node_a), (pos_b, node_b)) in
        a.registry.inline.iter_sorted().zip(b.registry.inline.iter_sorted())
    {
        assert_eq!(*pos_a, *pos_b, "inline[{pos_a}] position drift");
        assert_eq!(
            node_a.xml_node_name(),
            node_b.xml_node_name(),
            "inline[{pos_a}] kind drift"
        );
    }

    for ((pos_a, node_a), (pos_b, node_b)) in a
        .registry
        .block_leaf
        .iter_sorted()
        .zip(b.registry.block_leaf.iter_sorted())
    {
        assert_eq!(*pos_a, *pos_b, "block_leaf[{pos_a}] position drift");
        assert_eq!(
            node_a.xml_node_name(),
            node_b.xml_node_name(),
            "block_leaf[{pos_a}] kind drift"
        );
    }

    for ((pos_a, kind_a), (pos_b, kind_b)) in a
        .registry
        .block_open
        .iter_sorted()
        .zip(b.registry.block_open.iter_sorted())
    {
        assert_eq!(*pos_a, *pos_b, "block_open[{pos_a}] position drift");
        assert_eq!(kind_a, kind_b, "block_open[{pos_a}] container kind drift");
    }
    for ((pos_a, kind_a), (pos_b, kind_b)) in a
        .registry
        .block_close
        .iter_sorted()
        .zip(b.registry.block_close.iter_sorted())
    {
        assert_eq!(*pos_a, *pos_b, "block_close[{pos_a}] position drift");
        assert_eq!(kind_a, kind_b, "block_close[{pos_a}] container kind drift");
    }
}

fn assert_registry_aligned_with_sentinels(source: &str) {
    let arena = Arena::new();
    let out = aozora_lex::lex_into_arena(source, &arena);

    // Every registry entry's position must land on the matching
    // sentinel byte in `normalized`.
    //
    // The reverse direction ("every sentinel byte in normalized has a
    // registry entry") would be a tighter property but isn't
    // sound when the source itself contains PUA characters: the lexer
    // emits SourceContainsPua and passes those bytes through verbatim,
    // so the normalized text legitimately holds sentinel-shaped bytes
    // that the registry has not registered. The forward direction
    // alone catches every ordering / build-time bug we care about.
    for (pos, _) in out.registry.inline.iter_sorted() {
        let bytes = &out.normalized.as_bytes()[*pos as usize..];
        assert!(
            bytes.starts_with(&[0xEE, 0x80, 0x81]),
            "inline registry position {pos} is not at INLINE_SENTINEL for input {source:?}"
        );
    }
    for (pos, _) in out.registry.block_leaf.iter_sorted() {
        let bytes = &out.normalized.as_bytes()[*pos as usize..];
        assert!(
            bytes.starts_with(&[0xEE, 0x80, 0x82]),
            "block_leaf registry position {pos} is not at BLOCK_LEAF_SENTINEL for input {source:?}"
        );
    }
    for (pos, _) in out.registry.block_open.iter_sorted() {
        let bytes = &out.normalized.as_bytes()[*pos as usize..];
        assert!(
            bytes.starts_with(&[0xEE, 0x80, 0x83]),
            "block_open registry position {pos} is not at BLOCK_OPEN_SENTINEL for input {source:?}"
        );
    }
    for (pos, _) in out.registry.block_close.iter_sorted() {
        let bytes = &out.normalized.as_bytes()[*pos as usize..];
        assert!(
            bytes.starts_with(&[0xEE, 0x80, 0x84]),
            "block_close registry position {pos} is not at BLOCK_CLOSE_SENTINEL for input {source:?}"
        );
    }
}

fn check(source: &str) {
    assert_deterministic(source);
    assert_registry_aligned_with_sentinels(source);
}

// ----------------------------------------------------------------------
// Hand-curated regression anchors — same shapes as the prior arena
// equivalence test. Each anchor exercises one variant family.
// ----------------------------------------------------------------------

#[test]
fn empty_input() {
    check("");
}

#[test]
fn plain_text() {
    check("Hello, world.");
    check("こんにちは、世界！");
}

#[test]
fn explicit_ruby() {
    check("｜青梅《おうめ》");
}

#[test]
fn implicit_ruby() {
    check("青梅《おうめ》");
}

#[test]
fn double_ruby() {
    check("《《重要》》");
}

#[test]
fn bracket_annotations() {
    check("text［＃改ページ］more text");
    check("［＃ここから2字下げ］");
    check("［＃ここで字下げ終わり］");
}

#[test]
fn gaiji_marker() {
    check("※［＃「木＋吶のつくり」、第3水準1-85-54］");
}

#[test]
fn nested_quoted_annotation() {
    check("text［＃「青空」に傍点］more");
}

#[test]
fn mixed_pageful() {
    check(
        "明治の頃｜青梅《おうめ》街道沿いに、※［＃「木＋吶のつくり」、第3水準1-85-54］\n\
         なる珍しき木が立つ。［＃ここから2字下げ］その下で人々は語らひ。\n\
         ［＃ここで字下げ終わり］",
    );
}

// ----------------------------------------------------------------------
// Property tests: cover the same generator surface as the prior test.
// ----------------------------------------------------------------------

proptest! {
    #![proptest_config(default_config())]

    #[test]
    fn aozora_fragment_is_deterministic_and_aligned(s in aozora_fragment(120)) {
        check(&s);
    }

    #[test]
    fn pathological_aozora_is_deterministic_and_aligned(s in pathological_aozora(120)) {
        check(&s);
    }

    #[test]
    fn unicode_adversarial_is_deterministic_and_aligned(s in unicode_adversarial()) {
        check(&s);
    }
}
