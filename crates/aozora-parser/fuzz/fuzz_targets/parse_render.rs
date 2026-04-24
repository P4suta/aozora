//! Fuzz target — `afm_parser::html::render_to_string` on arbitrary UTF-8.
//!
//! Arbitrary bytes are decoded as UTF-8 (invalid sequences skip this
//! iteration). The resulting source is pushed through
//! `render_to_string`; every always-on invariant predicate is then
//! asserted. Property-test coverage gives this the same shape
//! predicates as `tests/property_html_shape.rs`, scaled to
//! libFuzzer's coverage-guided mutations for inputs the proptest
//! strategies don't reach.
//!
//! Tier A / Tier B are skipped — they have input preconditions (see
//! the property test's `assert_gated` helper) that would make this
//! target noisy on raw bytes.
//!
//! Run with: `just fuzz parse_render -- -runs=10000`

#![no_main]

use afm_parser::html::render_to_string;
use afm_parser::test_support::{
    check_annotation_wrapper_shape, check_content_model, check_css_class_contract,
    check_escape_invariants, check_heading_integrity, check_html_tag_balance,
    check_markup_completeness, check_no_xss_marker,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(src) = core::str::from_utf8(data) else {
        return;
    };
    let html = render_to_string(src);
    check_html_tag_balance(&html).expect("Tier D violated");
    check_annotation_wrapper_shape(&html).expect("Tier E violated");
    check_no_xss_marker(&html).expect("Tier F violated");
    check_css_class_contract(&html).expect("Tier G violated");
    check_escape_invariants(&html).expect("Tier I violated");
    check_content_model(&html).expect("Tier J violated");
    check_markup_completeness(&html).expect("Tier K violated");
    check_heading_integrity(&html).expect("Tier C violated");
});
