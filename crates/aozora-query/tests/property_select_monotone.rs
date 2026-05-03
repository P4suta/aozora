//! Selector monotonicity: a more permissive query selects a superset
//! of a more restrictive one.
//!
//! Drives the same intuition the tree-sitter query language gives: a
//! match on `(_)` (any node) must include every match on a specific
//! kind. A regression that silently drops captures from `(_)` would
//! make editor surfaces (LSP highlight, "find all ruby") miss
//! results — the proptest is the decisive killer.
//!
//! Two angles:
//!
//! 1. `(_)` captures every node a specific-kind query captures
//!    (subset relation).
//! 2. Two independent runs of `captures` on the same query + tree
//!    yield the same result count (determinism).

use aozora::Document;
use aozora::cst::{SyntaxNode, from_tree};
use aozora_proptest::config::default_config;
use aozora_proptest::generators::*;
use aozora_query::compile;
use proptest::prelude::*;

fn cst_for(source: &str) -> SyntaxNode {
    let doc = Document::new(source);
    let tree = doc.parse();
    from_tree(&tree)
}

fn assert_any_captures_superset_of_specific(source: &str) {
    let cst = cst_for(source);
    let any_query = compile("(_ @any)").expect("`(_ @any)` is a valid pattern");
    let construct_query = compile("(Construct @c)").expect("`(Construct @c)` is a valid pattern");

    let any_caps = any_query.captures(&cst);
    let construct_caps = construct_query.captures(&cst);

    // Every Construct capture's node must also appear in the any
    // capture set. We compare on text_range — `NodeOrToken` doesn't
    // implement Hash/Eq cheaply for set comparison.
    let any_ranges: Vec<_> = any_caps
        .iter()
        .map(|c| match &c.node {
            rowan::NodeOrToken::Node(n) => n.text_range(),
            rowan::NodeOrToken::Token(t) => t.text_range(),
        })
        .collect();

    for c in &construct_caps {
        let r = match &c.node {
            rowan::NodeOrToken::Node(n) => n.text_range(),
            rowan::NodeOrToken::Token(t) => t.text_range(),
        };
        assert!(
            any_ranges.contains(&r),
            "(_) failed to capture a node that (Construct) captured at range {r:?} for source {source:?}"
        );
    }
}

fn assert_capture_count_is_deterministic(source: &str) {
    let cst = cst_for(source);
    let q = compile("(_ @any)").expect("compile");
    let first = q.captures(&cst).len();
    let second = q.captures(&cst).len();
    assert_eq!(first, second, "captures count drift for source {source:?}");
}

// ----------------------------------------------------------------------
// Hand-curated regression anchors.
// ----------------------------------------------------------------------

#[test]
fn empty_input_yields_consistent_captures() {
    assert_any_captures_superset_of_specific("");
    assert_capture_count_is_deterministic("");
}

#[test]
fn ruby_input_captures_consistent() {
    assert_any_captures_superset_of_specific("｜青梅《おうめ》");
    assert_capture_count_is_deterministic("｜青梅《おうめ》");
}

#[test]
fn paired_container_captures_consistent() {
    assert_any_captures_superset_of_specific(
        "［＃ここから2字下げ］\nbody\n［＃ここで字下げ終わり］",
    );
}

proptest! {
    #![proptest_config(default_config())]

    /// `(_)` is a superset of `(Construct)` over the workhorse Aozora
    /// fragment generator — input shape independence.
    #[test]
    fn aozora_fragment_any_superset_of_construct(s in aozora_fragment(120)) {
        assert_any_captures_superset_of_specific(&s);
    }

    /// Capture count is deterministic — running the same query twice
    /// against the same CST returns the same number of captures.
    #[test]
    fn aozora_fragment_captures_count_is_deterministic(s in aozora_fragment(120)) {
        assert_capture_count_is_deterministic(&s);
    }

    /// Pathological / unbalanced inputs — query execution stays
    /// total even when the lex pipeline emits diagnostics.
    #[test]
    fn pathological_input_captures_consistent(s in pathological_aozora(120)) {
        assert_any_captures_superset_of_specific(&s);
    }
}
