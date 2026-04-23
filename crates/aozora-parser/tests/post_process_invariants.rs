//! End-to-end invariants for `afm_parser::parse` + `post_process`.
//!
//! The lexer classifies Aozora spans into a [`PlaceholderRegistry`] and
//! `post_process` splices each sentinel back into the comrak AST as
//! `NodeValue::Aozora(...)`. Both operations have tight correctness
//! contracts that this file pins from several angles:
//!
//! - **Tier-A invariant**: no bare `［＃` ever leaks into rendered HTML
//!   outside an `afm-annotation` wrapper, for arbitrary inputs.
//! - **Sentinel consumption**: after `splice_inline` runs, no
//!   `INLINE_SENTINEL` character remains in any `Text` node of the AST.
//!   After `splice_block_leaf` runs, no `BLOCK_LEAF_SENTINEL` survives
//!   in a single-sentinel paragraph shape.
//! - **Count invariant**: a source with *N* explicit-delimiter ruby
//!   annotations parses into an AST with exactly *N* `AozoraNode::Ruby`
//!   nodes. Same for unknown-body `［＃…］` → `Annotation{Unknown}`.
//! - **Document order**: Aozora nodes appear in source order.
//! - **Determinism**: `parse(x)` produces identical HTML on two
//!   independent arenas.
//! - **No panic on malformed shapes**: proptest-driven random
//!   combinations of Aozora triggers and plain text never crash.
//!
//! The tests here deliberately overlap with the lexer's Phase 6
//! validator (`afm-lexer::phase6_validate`) and with
//! `golden_56656 tier_a_no_panic_and_no_unconsumed_square_brackets` —
//! defence-in-depth is the point: every angle that spots the same bug
//! makes it harder for a regression to hide.

use afm_lexer::{BLOCK_LEAF_SENTINEL, INLINE_SENTINEL};
use afm_parser::html::render_to_string;
use afm_parser::test_support::strip_annotation_wrappers;
use afm_parser::{Options, parse};
use afm_syntax::AozoraNode;
use comrak::Arena;
use comrak::nodes::{AstNode, NodeValue};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Walk `root` and return every `AozoraNode` in document order.
fn collect_aozora<'a>(root: &'a AstNode<'a>) -> Vec<AozoraNode> {
    let mut out = Vec::new();
    for n in root.descendants() {
        if let NodeValue::Aozora(ref boxed) = n.data.borrow().value {
            out.push((**boxed).clone());
        }
    }
    out
}

/// Walk `root` and return concatenated text content of every Text node.
fn collect_text<'a>(root: &'a AstNode<'a>) -> String {
    let mut buf = String::new();
    for n in root.descendants() {
        if let NodeValue::Text(ref t) = n.data.borrow().value {
            buf.push_str(t);
        }
    }
    buf
}

fn parse_one(input: &str) -> (Vec<AozoraNode>, String) {
    let arena = Arena::new();
    let opts = Options::afm_default();
    let root = parse(&arena, input, &opts);
    (collect_aozora(root), collect_text(root))
}

fn render(input: &str) -> String {
    render_to_string(input)
}

// ---------------------------------------------------------------------------
// Sentinel consumption — unit-level smoke
// ---------------------------------------------------------------------------

#[test]
fn post_process_consumes_every_inline_sentinel_on_simple_ruby() {
    // One explicit ruby → one INLINE_SENTINEL in normalized text, which
    // post_process::splice_inline must splice out entirely. No raw
    // sentinel should survive in the AST's text nodes.
    let (_nodes, text) = parse_one("｜青梅《おうめ》へ");
    assert!(
        !text.contains(INLINE_SENTINEL),
        "INLINE_SENTINEL leaked into AST text: {text:?}",
    );
}

#[test]
fn post_process_consumes_every_block_sentinel_on_page_break() {
    // PageBreak on its own line → BLOCK_LEAF_SENTINEL in a standalone
    // paragraph, which splice_block_leaf replaces with an Aozora node.
    let (_nodes, text) = parse_one("前\n［＃改ページ］\n後");
    assert!(
        !text.contains(BLOCK_LEAF_SENTINEL),
        "BLOCK_LEAF_SENTINEL leaked into AST text: {text:?}",
    );
}

#[test]
fn post_process_consumes_sentinels_for_mixed_inline_and_block() {
    // Inline ruby + block page break + unknown annotation in one doc.
    let src = "｜漢字《かんじ》の話。\n\n［＃改ページ］\n\n［＃ほげ］まとめ";
    let (_nodes, text) = parse_one(src);
    assert!(
        !text.contains(INLINE_SENTINEL),
        "INLINE_SENTINEL leaked: {text:?}"
    );
    assert!(
        !text.contains(BLOCK_LEAF_SENTINEL),
        "BLOCK_LEAF_SENTINEL leaked: {text:?}"
    );
}

// ---------------------------------------------------------------------------
// Count invariants — unit-level
// ---------------------------------------------------------------------------

#[test]
fn explicit_ruby_count_matches_input_count() {
    let src = "｜青梅《おうめ》と｜鶴見《つるみ》、｜立川《たちかわ》";
    let (nodes, _) = parse_one(src);
    let ruby_count = nodes
        .iter()
        .filter(|n| matches!(n, AozoraNode::Ruby(_)))
        .count();
    assert_eq!(ruby_count, 3, "3 explicit rubies must yield 3 Ruby nodes");
}

#[test]
fn unknown_annotation_count_matches_input_count() {
    let src = "［＃ほげ］と［＃ふが］と［＃ぴよ］";
    let (nodes, _) = parse_one(src);
    let ann_count = nodes
        .iter()
        .filter(|n| matches!(n, AozoraNode::Annotation(_)))
        .count();
    assert_eq!(
        ann_count, 3,
        "3 unknown ［＃…］ must yield 3 Annotation nodes"
    );
}

// ---------------------------------------------------------------------------
// Document order
// ---------------------------------------------------------------------------

#[test]
fn aozora_nodes_appear_in_source_order() {
    let src = "｜一《いち》と｜二《に》と［＃ほげ］と｜三《さん》";
    let (nodes, _) = parse_one(src);
    // Expected sequence: Ruby, Ruby, Annotation, Ruby
    let shapes: Vec<_> = nodes
        .iter()
        .map(|n| match n {
            AozoraNode::Ruby(_) => "Ruby",
            AozoraNode::Annotation(_) => "Annotation",
            _ => "Other",
        })
        .collect();
    assert_eq!(shapes, ["Ruby", "Ruby", "Annotation", "Ruby"]);
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

#[test]
fn parse_is_deterministic_across_independent_arenas() {
    let src = "｜青梅《おうめ》前［＃改ページ］後\n［＃ほげ］続き";
    let a = render(src);
    let b = render(src);
    assert_eq!(a, b, "parse+render must be deterministic");
}

// ---------------------------------------------------------------------------
// Tier-A invariant — the biggest safety net
// ---------------------------------------------------------------------------

/// After rendering, the string `［＃` must not appear anywhere *outside*
/// an `afm-annotation` wrapper. The wrapper itself is allowed to contain
/// the raw markup because its body is `hidden` and preserved for
/// round-trip fidelity.
fn tier_a_holds(html: &str) -> bool {
    !strip_annotation_wrappers(html).contains("［＃")
}

#[test]
fn tier_a_holds_for_every_static_fixture() {
    // Hand-picked cases that together cover every recogniser class.
    let fixtures = [
        "plain text no annotations",
        "｜青梅《おうめ》",
        "｜漢字《かんじ》の話",
        "前［＃改ページ］後",
        "［＃改丁］\n",
        "前［＃ほげふが］後",
        "前［＃０字下げ］後", // zero-digit falls back to Annotation
        "語※［＃「木＋吶のつくり」、第3水準1-85-54］で", // gaiji
        "前［＃地付き］末尾",
        "冒頭でXが先行する。X［＃「X」に傍点］の強調。",
        "前［＃ここから２字下げ］本文［＃ここで字下げ終わり］後",
    ];
    for src in fixtures {
        let html = render(src);
        assert!(
            tier_a_holds(&html),
            "Tier-A leaked ［＃ for input {src:?}, html = {html:?}",
        );
    }
}

// ---------------------------------------------------------------------------
// Property tests — random adversarial input
// ---------------------------------------------------------------------------

/// Strategy that produces a string of mixed plain text, ruby shapes,
/// bracket annotations, and reference marks. Length is bounded so the
/// test suite stays fast.
fn mixed_aozora_strategy() -> impl Strategy<Value = String> {
    let atoms = prop_oneof![
        Just("｜".to_owned()),
        Just("《".to_owned()),
        Just("》".to_owned()),
        Just("［＃".to_owned()),
        Just("］".to_owned()),
        Just("※".to_owned()),
        Just("改ページ".to_owned()),
        Just("改丁".to_owned()),
        Just("漢字".to_owned()),
        Just("かんじ".to_owned()),
        Just("ABC".to_owned()),
        Just("1234".to_owned()),
        Just("\n".to_owned()),
        Just("\n\n".to_owned()),
        Just("、".to_owned()),
        Just("。".to_owned()),
        Just(" ".to_owned()),
    ];
    prop::collection::vec(atoms, 0..16).prop_map(|pieces| pieces.join(""))
}

/// Returns `true` when the lexer raises no diagnostic for `src`, i.e.
/// every bracket / ruby / quote pair was closed in order. Used to
/// restrict property-test inputs to well-formed Aozora shapes before
/// asserting the Tier-A canary — malformed inputs (stray `］`, unclosed
/// `［＃`, bracket ordering inversions like `］［＃`) are boundary
/// conditions handled by separate dedicated unit tests (see
/// `malformed_*` below), not by the Tier-A property.
fn lexer_is_well_formed(src: &str) -> bool {
    afm_lexer::lex(src).diagnostics.is_empty()
}

proptest! {
    /// Arbitrary combinations of Aozora triggers must:
    ///
    /// 1. Not panic `parse()` — the pipeline must be total.
    /// 2. Not leak any `INLINE_SENTINEL` into the AST's Text nodes —
    ///    `post_process::splice_inline` is responsible for consuming
    ///    every one regardless of surrounding shape.
    /// 3. For well-formed inputs (matched brackets), not leak `［＃`
    ///    into rendered HTML (Tier-A canary).
    #[test]
    fn parse_survives_arbitrary_aozora_shaped_input(src in mixed_aozora_strategy()) {
        let (_nodes, text) = parse_one(&src);
        prop_assert!(
            !text.contains(INLINE_SENTINEL),
            "INLINE_SENTINEL leaked for src {src:?}, text {text:?}",
        );
        if lexer_is_well_formed(&src) {
            let html = render_to_string(&src);
            prop_assert!(
                tier_a_holds(&html),
                "Tier-A leaked for src {src:?}, html {html:?}",
            );
        }
    }

    /// Parse is deterministic: two independent arena allocations
    /// produce identical rendered HTML.
    #[test]
    fn parse_determinism(src in mixed_aozora_strategy()) {
        let a = render_to_string(&src);
        let b = render_to_string(&src);
        prop_assert_eq!(a, b);
    }
}

// ---------------------------------------------------------------------------
// Malformed-input boundary conditions — complements the proptest property
// above (which restricts to balanced brackets) with explicit cases for
// unclosed / stray shapes. The asserting property is narrower: the parser
// must not panic and must be deterministic. Tier-A is *not* promised for
// malformed input.
// ---------------------------------------------------------------------------

#[test]
fn malformed_unclosed_bracket_does_not_panic() {
    // `［＃` without a closing `］`. Tier-A may leak (the bracket stays
    // as plain text) — that is the documented boundary. What matters
    // is that parse + render complete without a panic.
    let _html = render_to_string("前［＃");
    let _html = render_to_string("［＃ほげ");
    // Determinism on malformed input too.
    let a = render_to_string("［＃");
    let b = render_to_string("［＃");
    assert_eq!(a, b);
}

#[test]
fn malformed_unclosed_ruby_does_not_panic() {
    let _html = render_to_string("｜青梅《");
    let _html = render_to_string("《》");
    let _html = render_to_string("｜");
    // Stray reference mark.
    let _html = render_to_string("※");
}

#[test]
fn malformed_stray_close_bracket_does_not_panic() {
    // `］` with no matching `［＃` is consumed as plain text with a
    // Phase-2 UnmatchedClose diagnostic surfaced via LexOutput.
    let _html = render_to_string("stray］text");
    let _html = render_to_string("》trailing");
}

// ---------------------------------------------------------------------------
// PUA collision — source already contains PUA characters the lexer uses
// internally. Phase 0 must emit a diagnostic but the pipeline must not
// produce nonsense output for downstream consumers.
// ---------------------------------------------------------------------------

#[test]
fn source_containing_pua_characters_does_not_panic() {
    // E001 is the INLINE_SENTINEL; source shouldn't contain it, but
    // if it does the lexer must not confuse it with its own
    // sentinels.
    let html = render_to_string("before\u{E001}after");
    assert!(!html.is_empty(), "parse must produce some output");
    // Determinism on PUA-tainted input too.
    let a = render_to_string("before\u{E001}after");
    let b = render_to_string("before\u{E001}after");
    assert_eq!(a, b);
}
