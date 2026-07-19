//! Lossless invariant — the central CST property: leaf-text concatenation must
//! equal the sanitized source bytes for every input the classifier accepts.
//!
//! This property is what justifies rowan over a hand-rolled tree and what
//! enables source-faithful tooling (formatters, refactoring, comment-preserving
//! rewrites). A regression here breaks every editor surface that walks the CST.
#![cfg(feature = "cst")]

use aozora::Document;
use aozora::cst::from_snapshot;
use aozora_proptest::config::default_config;
use aozora_proptest::generators::{aozora_fragment, pathological_aozora, unicode_adversarial};
use proptest::prelude::*;

/// Returns `(sanitized_source, cst_reconstruction)`. The lossless invariant
/// asserts the two are equal.
fn reconstruct_sanitized(src: &str) -> (String, String) {
    let doc = Document::new(src);
    let tree = doc.snapshot();
    let cst = from_snapshot(&tree);
    let mut buf = String::new();
    for step in cst.preorder_with_tokens() {
        if let rowan::WalkEvent::Enter(rowan::NodeOrToken::Token(t)) = step {
            buf.push_str(t.text());
        }
    }
    (tree.sanitized().to_owned(), buf)
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
