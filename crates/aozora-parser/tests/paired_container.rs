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
fn warichu_renders_inline_not_as_block_container() {
    // Aozora spec: `［＃割り注］…［＃割り注終わり］` is inline (a
    // small-text side-note flowing with the surrounding prose). The
    // renderer must emit a single `<span class="afm-warichu">` pair
    // inside the host paragraph — *not* a block `<div>` that would
    // split the sentence mid-stream (as the deprecated
    // `ここから割り注` form used to). This test pins the fix for the
    // 56656 `黄色い鑑札（…）` rendering bug.
    let html = render_to_string("黄色い鑑札（［＃割り注］淫売婦の鑑札［＃割り注終わり］）をもって");
    assert!(
        html.contains(r#"<span class="afm-warichu">淫売婦の鑑札</span>"#),
        "warichu must render as inline span: {html}"
    );
    assert!(
        !html.contains(r#"<div class="afm-container afm-container-warichu">"#),
        "warichu must not render as block container: {html}"
    );
    // Host paragraph stays intact — the full sentence is one <p>.
    assert_eq!(
        html.matches("<p>").count(),
        1,
        "warichu must not split the host paragraph: {html}"
    );
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

// ---------------------------------------------------------------------------
// Same-family implicit close (Aozora spec convention)
//
// The Aozora annotation spec for 字下げ / 地付き
// (<https://www.aozora.gr.jp/annotation/indent.html>) asks that a
// second `ここから…` of the same family implicitly ends the previous
// scope — they are state-changing, not stack-nesting. 罪と罰 (fixture
// 56656) exercises this shape around the Malborough song and was
// leaking a bare U+E003 (BLOCK_OPEN_SENTINEL) into the rendered HTML
// until Phase 5's post_process fix.
// ---------------------------------------------------------------------------

#[test]
fn consecutive_indent_opens_with_single_close_do_not_leak_sentinel() {
    // Exact shape from 罪と罰 around offset 1.6 MB. Two ［＃ここから…
    // 字下げ］ opens of different amounts with only one explicit close
    // between them. Per Aozora spec the inner open implicitly ends the
    // outer scope so both render as sibling containers.
    let html = render_to_string(
        "［＃ここから２字下げ］\n\
         前半一行目\n\
         前半二行目\n\
         ［＃ここから５字下げ］\n\
         後半一行目\n\
         後半二行目\n\
         ［＃ここで字下げ終わり］",
    );
    assert_tier_a(&html);
    // Both indent scopes must materialise as containers.
    assert!(
        html.contains("afm-container-indent-2"),
        "outer Indent{{2}} must not be lost: {html}"
    );
    assert!(
        html.contains("afm-container-indent-5"),
        "inner Indent{{5}} must render: {html}"
    );
    // No orphan sentinel survives — `assert_tier_a` already checks,
    // but pin the specific codepoint so a regression surfaces under
    // the exact lexer constant name.
    assert!(
        !html.contains('\u{E003}'),
        "BLOCK_OPEN_SENTINEL (U+E003) leaked: {html}"
    );
}

#[test]
fn three_consecutive_same_family_opens_with_one_close_all_wrap() {
    // Three cascading opens followed by a single explicit close. Each
    // new open implicitly closes the previous; the explicit close
    // matches the last open.
    let html = render_to_string(
        "［＃ここから１字下げ］\n\
         A\n\
         ［＃ここから２字下げ］\n\
         B\n\
         ［＃ここから３字下げ］\n\
         C\n\
         ［＃ここで字下げ終わり］",
    );
    assert_tier_a(&html);
    for amount in [1, 2, 3] {
        assert!(
            html.contains(&format!("afm-container-indent-{amount}")),
            "Indent{{{amount}}} scope must render: {html}"
        );
    }
    assert!(!html.contains('\u{E003}'), "no U+E003 may survive: {html}");
    assert!(!html.contains('\u{E004}'), "no U+E004 may survive: {html}");
}

#[test]
fn cross_family_nest_preserved_only_same_family_cascades() {
    // Keigakomi and Indent are different families — the inner Indent
    // must NOT implicitly close the surrounding Keigakomi. Verifies
    // the same_family predicate is not over-aggressive.
    //
    // Keigakomi's paired syntax is `［＃罫囲み］...［＃罫囲み終わり］`
    // (no `ここから` / `ここで` prefix — the classifier accepts the
    // bare form per phase3_classify).
    let html = render_to_string(
        "［＃罫囲み］\n\
         外枠テキスト\n\
         ［＃ここから２字下げ］\n\
         内側テキスト\n\
         ［＃ここで字下げ終わり］\n\
         戻り\n\
         ［＃罫囲み終わり］",
    );
    assert_tier_a(&html);
    assert!(
        html.contains("afm-container-keigakomi"),
        "outer Keigakomi must render: {html}"
    );
    assert!(
        html.contains("afm-container-indent-2"),
        "nested Indent{{2}} must render: {html}"
    );
    // The keigakomi <div> must open BEFORE the indent <div> —
    // nested containers preserve the source ordering.
    let keigakomi_open = html.find("afm-container-keigakomi").unwrap();
    let indent_open = html.find("afm-container-indent-2").unwrap();
    assert!(
        indent_open > keigakomi_open,
        "Indent must open inside the Keigakomi: {html}"
    );
}

#[test]
fn same_family_cascade_preserves_align_end_shape() {
    // 地付き (AlignEnd) family — two consecutive opens should cascade
    // the same way Indent does.
    let html = render_to_string(
        "［＃ここから地付き］\n\
         後書き一行目\n\
         ［＃ここから地から３字上げ］\n\
         後書き二行目\n\
         ［＃ここで地付き終わり］",
    );
    assert_tier_a(&html);
    assert!(
        html.contains("afm-container-align-end"),
        "AlignEnd must render: {html}"
    );
    assert!(!html.contains('\u{E003}'), "no U+E003 may survive: {html}");
}
