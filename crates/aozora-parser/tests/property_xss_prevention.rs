//! Property test — Tier F: no XSS marker reaches the rendered HTML.
//!
//! afm's default options set `render.unsafe_ = false`, so every raw
//! HTML block in the input must be neutralised — `<script>alert(1)</script>`
//! on the input side must not survive as `<script>` on the output
//! side. This property drives curated dangerous payloads through
//! `render_to_string` and asserts that
//! [`check_no_xss_marker`](aozora_parser::test_support::check_no_xss_marker)
//! succeeds: no raw `<script`, no `javascript:` URI in attribute
//! values, no `on<event>=` attribute handler.
//!
//! # Coverage breadth
//!
//! Three proptests:
//!
//! 1. Canned payloads alone (20+ hand-picked dangerous strings).
//! 2. Canned payloads wrapped in Aozora markup — exercises the
//!    interaction between comrak's raw-HTML neutralisation and afm's
//!    annotation-wrapper rendering.
//! 3. Canned payloads inside adversarial CommonMark (lists,
//!    blockquotes, tables). Ensures no raw-HTML passthrough leaks via
//!    any CommonMark construct.
//!
//! # Why this file is separate
//!
//! A failure here is a *security bug*; keeping it in its own file
//! makes the test name loud in CI logs (`XSS prevention failed`
//! rather than `html_shape_invariants failed on one of ten
//! predicates`). The generator (`xss_payload`) is also disjoint in
//! shape from the Aozora fragment generators — combining them into
//! one file would dilute shrinking effectiveness.

use aozora_parser::html::render_to_string;
use aozora_parser::test_support::check_no_xss_marker;
use aozora_test_utils::config::default_config;
use aozora_test_utils::generators::{aozora_fragment, commonmark_adversarial, xss_payload};
use proptest::prelude::*;

proptest! {
    #![proptest_config(default_config())]

    /// Canned dangerous payloads alone. Every payload must be
    /// neutralised on render.
    #[test]
    fn xss_payload_alone_is_neutralised(payload in xss_payload()) {
        let html = render_to_string(&payload);
        check_no_xss_marker(&html)
            .unwrap_or_else(|e| panic!("XSS leak for payload={payload:?}: {e}"));
    }

    /// Payload sandwiched between Aozora markup. If the annotation
    /// wrapper or ruby emitter accidentally leaves a pass-through gap,
    /// this catches it.
    #[test]
    fn xss_payload_wrapped_in_aozora_is_neutralised(
        before in aozora_fragment(4),
        payload in xss_payload(),
        after in aozora_fragment(4),
    ) {
        let src = format!("{before}{payload}{after}");
        let html = render_to_string(&src);
        check_no_xss_marker(&html)
            .unwrap_or_else(|e| panic!("XSS leak for src={src:?}: {e}"));
    }

    /// Payload inside adversarial CommonMark (blockquote + list +
    /// heading, tight vs loose lists, backslash escapes). Catches
    /// raw-HTML passthrough via any CommonMark construct.
    #[test]
    fn xss_payload_in_adversarial_cm_is_neutralised(
        prefix in commonmark_adversarial(),
        payload in xss_payload(),
    ) {
        let src = format!("{prefix}\n\n{payload}");
        let html = render_to_string(&src);
        check_no_xss_marker(&html)
            .unwrap_or_else(|e| panic!("XSS leak for src={src:?}: {e}"));
    }
}
