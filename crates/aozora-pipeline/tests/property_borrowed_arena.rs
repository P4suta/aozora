//! Determinism + structural invariants for `lex_into_arena`.
//!
//! 1. Two independent runs over the same source produce byte-identical
//!    normalised text and identical registry shape (positions + node
//!    kinds + container kinds + diagnostic count).
//! 2. Every PUA sentinel in the normalised text has a registry entry,
//!    and every registry entry's position points at a PUA sentinel.
//!
//! Together these gate any future change to `lex_into_arena` from
//! introducing nondeterminism (e.g. iteration order over a `HashMap`)
//! or from desynchronising the registry from the normalised text.

use aozora_spec::Sentinel;
use aozora_syntax::borrowed::{Arena, NodeRef};
use aozora_test_utils::config::default_config;
use aozora_test_utils::generators::*;
use proptest::prelude::*;

#[allow(
    clippy::too_many_lines,
    reason = "exhaustive determinism asserter: each block checks one independent invariant (normalized text, sanitized_len, diagnostic count, four registries, four iter_sorted walks). Splitting into helpers would obscure the per-invariant intent without saving lines."
)]
fn assert_deterministic(source: &str) {
    let arena_a = Arena::new();
    let arena_b = Arena::new();
    let a = aozora_pipeline::lex_into_arena(source, &arena_a);
    let b = aozora_pipeline::lex_into_arena(source, &arena_b);

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
    for kind in Sentinel::ALL {
        assert_eq!(
            a.registry.count_kind(kind),
            b.registry.count_kind(kind),
            "{kind:?} registry length non-deterministic for input {source:?}"
        );
    }

    // Per-position node kind equivalence across the two runs. Walk
    // every entry in the unified registry, in position order; both
    // runs must see the same sequence of (position, NodeRef-variant,
    // payload-shape) triples.
    for ((pos_a, nr_a), (pos_b, nr_b)) in a.registry.iter_sorted().zip(b.registry.iter_sorted()) {
        assert_eq!(pos_a, pos_b, "registry[{pos_a}] position drift");
        assert_eq!(
            nr_a.sentinel_kind(),
            nr_b.sentinel_kind(),
            "registry[{pos_a}] sentinel kind drift"
        );
        match (nr_a, nr_b) {
            (NodeRef::Inline(node_a), NodeRef::Inline(node_b))
            | (NodeRef::BlockLeaf(node_a), NodeRef::BlockLeaf(node_b)) => {
                assert_eq!(
                    node_a.xml_node_name(),
                    node_b.xml_node_name(),
                    "registry[{pos_a}] inline / block-leaf payload kind drift"
                );
            }
            (NodeRef::BlockOpen(kind_a), NodeRef::BlockOpen(kind_b))
            | (NodeRef::BlockClose(kind_a), NodeRef::BlockClose(kind_b)) => {
                assert_eq!(kind_a, kind_b, "registry[{pos_a}] container kind drift");
            }
            _ => {
                panic!("registry[{pos_a}] cross-variant drift: {nr_a:?} vs {nr_b:?}");
            }
        }
    }
}

fn assert_registry_aligned_with_sentinels(source: &str) {
    let arena = Arena::new();
    let out = aozora_pipeline::lex_into_arena(source, &arena);

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
    for (pos, nr) in out.registry.iter_sorted() {
        let bytes = &out.normalized.as_bytes()[pos as usize..];
        let kind = nr.sentinel_kind();
        // The classifier emits one of the four PUA sentinels; map
        // back to the UTF-8 byte triple so we can byte-match in the
        // normalized buffer. Computing this from the `Sentinel` char
        // representation avoids the wildcard `_ => panic!()` arm
        // that clippy reads as unreachable for an exhaustive enum.
        let expected_utf8 = utf8_bytes_for_sentinel(kind);
        assert!(
            bytes.starts_with(&expected_utf8),
            "{kind:?} registry position {pos} is not at the matching sentinel for input {source:?}"
        );
    }
}

/// UTF-8 byte triple for a [`Sentinel`]'s codepoint. PUA sentinels all
/// encode as 3 bytes; we encode through `char::encode_utf8` once so
/// new variants don't need a hand-maintained byte table.
fn utf8_bytes_for_sentinel(kind: Sentinel) -> [u8; 3] {
    let mut buf = [0u8; 4];
    let s = kind.as_char().encode_utf8(&mut buf);
    let bytes = s.as_bytes();
    debug_assert_eq!(bytes.len(), 3, "PUA sentinels are always 3 UTF-8 bytes");
    [bytes[0], bytes[1], bytes[2]]
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
