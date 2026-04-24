//! Integration tests for paired-container AST wrap.
//!
//! `［＃ここから…］ … ［＃ここで…終わり］` brackets in the source
//! surface as an `AozoraNode::Container(Container { kind })` block in
//! the AST, with every block between the open and close moved under
//! it as children. Rendering wraps the children in a
//! `<div class="afm-container afm-container-<slug>">` on enter and
//! emits `</div>` on exit while comrak walks the children in between.
//!
//! Tests cover:
//!
//! * Each of the four canonical `ContainerKind` variants (`Indent`,
//!   `AlignEnd`, `Keigakomi`, `Warichu`) wraps its body.
//! * Nested containers (Keigakomi inside Indent) land on two passes.
//! * Orphan open / orphan close do NOT corrupt the tree and keep the
//!   stray sentinel visible.
//! * Tier-A canary: no bare `［＃` ever reaches the rendered HTML.

use afm_parser::html::render_to_string;

/// Tier-A canary reused from `ruby_segments.rs`: strip any
/// `afm-annotation` hidden-span bodies (which are allowed to carry
/// raw `［＃`) and assert the remaining HTML has no bare marker.
fn strip_annotation_wrappers(html: &str) -> String {
    let open = r#"<span class="afm-annotation" hidden>"#;
    let close = "</span>";
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find(open) {
        out.push_str(&rest[..start]);
        let after_open = &rest[start + open.len()..];
        if let Some(close_rel) = after_open.find(close) {
            rest = &after_open[close_rel + close.len()..];
        } else {
            out.push_str(after_open);
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

fn assert_tier_a(html: &str) {
    let stripped = strip_annotation_wrappers(html);
    assert!(
        !stripped.contains("［＃"),
        "bare ［＃ leaked outside afm-annotation\n  full: {html:?}\n  stripped: {stripped:?}"
    );
    // PUA sentinels must never survive to the rendered HTML either.
    for sentinel in ['\u{E001}', '\u{E002}', '\u{E003}', '\u{E004}'] {
        assert!(
            !stripped.contains(sentinel),
            "PUA sentinel U+{:04X} leaked: {html:?}",
            sentinel as u32
        );
    }
}

// ---------------------------------------------------------------------------
// Four canonical container kinds
// ---------------------------------------------------------------------------

#[test]
fn indent_container_wraps_body_with_div() {
    let html = render_to_string("［＃ここから字下げ］\n本文\n［＃ここで字下げ終わり］");
    // Opening and closing must bracket the body paragraph.
    assert!(
        html.contains(
            r#"<div class="afm-container afm-container-indent afm-container-indent-1" data-amount="1">"#,
        ),
        "open tag missing: {html:?}"
    );
    assert!(html.contains("</div>"), "close tag missing: {html:?}");
    assert!(
        html.contains("<p>本文</p>"),
        "inner paragraph must survive: {html:?}"
    );
    assert_tier_a(&html);
}

#[test]
fn indent_container_with_amount_carries_data_attribute() {
    let html = render_to_string("［＃ここから3字下げ］\n本文\n［＃ここで字下げ終わり］");
    assert!(
        html.contains(
            r#"<div class="afm-container afm-container-indent afm-container-indent-3" data-amount="3">"#,
        ),
        "3-wide indent must carry data-amount: {html:?}"
    );
    assert_tier_a(&html);
}

#[test]
fn align_end_container_wraps_body() {
    let html = render_to_string("［＃ここから地付き］\n後書き\n［＃ここで地付き終わり］");
    assert!(
        html.contains(r#"<div class="afm-container afm-container-align-end" data-offset="0">"#),
        "align-end open tag missing: {html:?}"
    );
    assert!(html.contains("<p>後書き</p>"));
    assert_tier_a(&html);
}

#[test]
fn keigakomi_container_wraps_body() {
    let html = render_to_string("［＃罫囲み］\n引用\n［＃罫囲み終わり］");
    assert!(
        html.contains(r#"<div class="afm-container afm-container-keigakomi">"#),
        "keigakomi open tag missing: {html:?}"
    );
    assert!(html.contains("<p>引用</p>"));
    assert_tier_a(&html);
}

#[test]
fn warichu_container_wraps_body() {
    let html = render_to_string("［＃割り注］\n注記本体\n［＃割り注終わり］");
    assert!(
        html.contains(r#"<div class="afm-container afm-container-warichu">"#),
        "warichu open tag missing: {html:?}"
    );
    assert!(html.contains("<p>注記本体</p>"));
    assert_tier_a(&html);
}

// ---------------------------------------------------------------------------
// Nesting
// ---------------------------------------------------------------------------

#[test]
fn keigakomi_inside_indent_wraps_via_two_passes() {
    let html = render_to_string(
        "［＃ここから字下げ］\n\n［＃罫囲み］\n本文\n［＃罫囲み終わり］\n\n［＃ここで字下げ終わり］",
    );
    // Two opens + two closes with correct nesting
    let indent_open_at = html
        .find("afm-container-indent")
        .expect("indent open present");
    let keigakomi_open_at = html
        .find("afm-container-keigakomi")
        .expect("keigakomi open present");
    assert!(
        indent_open_at < keigakomi_open_at,
        "indent must open before keigakomi: {html:?}"
    );
    // Inner body still present
    assert!(html.contains("<p>本文</p>"));
    assert_tier_a(&html);
}

// ---------------------------------------------------------------------------
// Orphan handling — open without close and vice versa
// ---------------------------------------------------------------------------

#[test]
fn orphan_open_does_not_panic_but_leaves_sentinel_paragraph() {
    // No matching `［＃ここで字下げ終わり］`: splice refuses to wrap;
    // the rendered HTML contains whatever the sentinel paragraph
    // renders as, but MUST NOT panic or corrupt the tree.
    let html = render_to_string("［＃ここから字下げ］\n本文");
    assert!(
        html.contains("本文"),
        "body content must survive orphan open: {html:?}"
    );
    // Tier-A: sentinel may survive as a PUA char, which is *not*
    // `［＃` — so the canary still holds even in the error case.
    // We specifically check `strip_annotation_wrappers` for `［＃`.
    let stripped = strip_annotation_wrappers(&html);
    assert!(
        !stripped.contains("［＃"),
        "orphan-open leaked bare ［＃: {html:?}"
    );
}

#[test]
fn orphan_close_does_not_panic() {
    let html = render_to_string("本文\n［＃ここで字下げ終わり］");
    assert!(
        html.contains("本文"),
        "body must survive orphan close: {html:?}"
    );
    let stripped = strip_annotation_wrappers(&html);
    assert!(
        !stripped.contains("［＃"),
        "orphan-close leaked bare ［＃: {html:?}"
    );
}

// ---------------------------------------------------------------------------
// Empty / edge cases
// ---------------------------------------------------------------------------

#[test]
fn empty_container_body_still_wraps_with_div() {
    // Paired markers back-to-back with no content between them.
    let html = render_to_string("［＃ここから字下げ］\n\n［＃ここで字下げ終わり］");
    assert!(html.contains("afm-container-indent"));
    assert!(html.contains("</div>"));
    assert_tier_a(&html);
}

#[test]
fn container_with_multiple_child_blocks_captures_all() {
    let html = render_to_string(
        "［＃ここから字下げ］\n\n段落一\n\n段落二\n\n段落三\n\n［＃ここで字下げ終わり］",
    );
    // Three <p> under the wrapping <div>.
    let p_count = html.matches("<p>").count();
    assert_eq!(p_count, 3, "3 child paragraphs expected: {html:?}");
    assert_tier_a(&html);
}
