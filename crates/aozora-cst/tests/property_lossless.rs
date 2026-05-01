//! Lossless invariant — the central CST property: leaf-text
//! concatenation must equal the original source bytes for every
//! input the classifier accepts.
//!
//! This property is what justifies rowan over a hand-rolled tree
//! and what enables source-faithful tooling (formatters,
//! refactoring, comment-preserving rewrites). A regression here
//! breaks every editor surface that walks the CST.

use aozora::Document;
use aozora::pipeline::lexer::sanitize;
use aozora_cst::build_cst;
use aozora_proptest::config::default_config;
use aozora_proptest::generators::{aozora_fragment, pathological_aozora, unicode_adversarial};
use proptest::prelude::*;

/// Returns `(sanitized_source, cst_reconstruction)`. The lossless
/// invariant asserts the two are equal.
fn reconstruct_sanitized(src: &str) -> (String, String) {
    let sanitized = sanitize(src);
    let doc = Document::new(src);
    let tree = doc.parse();
    let cst = build_cst(&sanitized.text, tree.source_nodes());
    let mut buf = String::with_capacity(sanitized.text.len());
    for step in cst.preorder_with_tokens() {
        if let rowan::WalkEvent::Enter(rowan::NodeOrToken::Token(t)) = step {
            buf.push_str(t.text());
        }
    }
    (sanitized.text.into_owned(), buf)
}

proptest! {
    #![proptest_config(default_config())]

    #[test]
    fn aozora_fragment_round_trips_through_cst(src in aozora_fragment(120)) {
        let (expected, actual) = reconstruct_sanitized(&src);
        prop_assert_eq!(actual, expected);
    }

    #[test]
    fn pathological_aozora_round_trips_through_cst(src in pathological_aozora(120)) {
        let (expected, actual) = reconstruct_sanitized(&src);
        prop_assert_eq!(actual, expected);
    }

    #[test]
    fn unicode_adversarial_round_trips_through_cst(src in unicode_adversarial()) {
        let (expected, actual) = reconstruct_sanitized(&src);
        prop_assert_eq!(actual, expected);
    }
}
