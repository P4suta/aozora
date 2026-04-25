//! Fuzz target — `aozora_encoding::decode_sjis` + full render pipeline.
//!
//! Arbitrary bytes are fed into `decode_sjis`. Failures (non-SJIS
//! input) skip this iteration. Successful decodes are pushed through
//! the full render pipeline and every always-on invariant predicate
//! is asserted. Targets encoding-boundary bugs (truncated trail
//! bytes, lead-byte-at-EOF, SJIS-adjacent codepoints that decode to
//! Aozora trigger glyphs after mapping).
//!
//! Run with: `just fuzz sjis_decode -- -runs=10000`

#![no_main]

use aozora_encoding::decode_sjis;
use aozora_parser::html::render_to_string;
use aozora_parser::test_support::{
    check_annotation_wrapper_shape, check_content_model, check_css_class_contract,
    check_escape_invariants, check_heading_integrity, check_html_tag_balance,
    check_markup_completeness, check_no_xss_marker,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = decode_sjis(data) else {
        return;
    };
    let html = render_to_string(&text);
    check_html_tag_balance(&html).expect("Tier D violated");
    check_annotation_wrapper_shape(&html).expect("Tier E violated");
    check_no_xss_marker(&html).expect("Tier F violated");
    check_css_class_contract(&html).expect("Tier G violated");
    check_escape_invariants(&html).expect("Tier I violated");
    check_content_model(&html).expect("Tier J violated");
    check_markup_completeness(&html).expect("Tier K violated");
    check_heading_integrity(&html).expect("Tier C violated");
});
