//! Interaction between Aozora constructs and CommonMark block
//! structures (list items, blockquotes, ATX headings, setext headings,
//! code fences, thematic breaks, nested containers).
//!
//! The lexer's block-leaf normalization pads `［＃改ページ］`-style
//! sentinels with blank lines (`\n\n`) so comrak treats them as
//! standalone paragraphs. That padding interacts non-trivially with
//! comrak's block-level parsers: a blank line inside a list item
//! terminates the list; a blank line inside a blockquote with a
//! `>`-continuation can change indentation; nesting amplifies both
//! effects.
//!
//! These tests document the CURRENT behaviour so any regression that
//! silently changes HTML shape around a common CommonMark construct
//! surfaces here. They also pin the Tier-A invariant (no bare `［＃`
//! leaking) across every shape.

use afm_parser::html::render_to_string;
use afm_parser::test_support::strip_annotation_wrappers;

/// Strip afm-annotation wrappers then assert no `［＃` survives.
/// Wrappers carry the raw markup inside a `hidden` span for round-trip
/// fidelity; outside them the output must be marker-free.
fn assert_tier_a(html: &str) {
    let stripped = strip_annotation_wrappers(html);
    assert!(
        !stripped.contains("［＃"),
        "Tier-A leaked ［＃ outside afm-annotation wrapper. Full HTML: {html:?}"
    );
}

// ---------------------------------------------------------------------------
// ATX headings
// ---------------------------------------------------------------------------

#[test]
fn heading_with_inline_ruby_renders_ruby_inside_heading() {
    let html = render_to_string("# ｜青梅《おうめ》について");
    assert!(
        html.starts_with("<h1>"),
        "heading must be rendered as <h1>, got {html:?}"
    );
    assert!(
        html.contains("<ruby>青梅"),
        "heading body must still carry the ruby tag, got {html:?}"
    );
    assert_tier_a(&html);
}

#[test]
fn heading_followed_by_page_break_separates_cleanly() {
    // The ATX heading is a single block; the block-leaf PageBreak
    // that follows is a sibling block.
    let html = render_to_string("# Chapter one\n\n［＃改ページ］\n\n# Chapter two");
    assert!(
        html.contains("<h1>Chapter one</h1>"),
        "first heading must render, got {html:?}"
    );
    assert!(
        html.contains(r#"<div class="afm-page-break"></div>"#),
        "page break must render as a sibling div, got {html:?}"
    );
    assert!(
        html.contains("<h1>Chapter two</h1>"),
        "second heading must render, got {html:?}"
    );
    assert_tier_a(&html);
}

// ---------------------------------------------------------------------------
// Blockquotes
// ---------------------------------------------------------------------------

#[test]
fn blockquote_with_inline_ruby_keeps_ruby_inside_block() {
    let html = render_to_string("> ｜青梅《おうめ》を引用\n> — 出典");
    assert!(
        html.contains("<blockquote>"),
        "blockquote must render, got {html:?}"
    );
    assert!(
        html.contains("<ruby>青梅"),
        "ruby must live inside the blockquote, got {html:?}"
    );
    assert_tier_a(&html);
}

#[test]
fn blockquote_with_forward_bouten_still_promotes() {
    // Target `X` appears inside the blockquote before the annotation —
    // should trigger the forward-ref bouten promotion.
    let html = render_to_string("> Xは重要\n>\n> X［＃「X」に傍点］に注目");
    assert!(
        html.contains("<blockquote>"),
        "blockquote must render, got {html:?}"
    );
    assert!(
        html.contains("afm-bouten"),
        "forward-ref bouten must promote inside blockquote, got {html:?}"
    );
    assert_tier_a(&html);
}

// ---------------------------------------------------------------------------
// List items
// ---------------------------------------------------------------------------

#[test]
fn list_item_with_inline_ruby_carries_ruby() {
    let html = render_to_string("- ｜一《いち》\n- ｜二《に》\n- ｜三《さん》");
    assert!(
        html.contains("<ul>") || html.contains("<ol>"),
        "list must render, got {html:?}"
    );
    // Three ruby tags, one per list item.
    assert_eq!(
        html.matches("<ruby>").count(),
        3,
        "each list item must carry its own ruby, got {html:?}"
    );
    assert_tier_a(&html);
}

#[test]
fn list_item_with_unknown_annotation_wraps_inside_item() {
    let html = render_to_string("- 一［＃ほげ］\n- 二［＃ふが］");
    // Each list item gets its own afm-annotation wrapper.
    assert_eq!(
        html.matches(r#"<span class="afm-annotation" hidden>"#)
            .count(),
        2,
        "each list item must wrap its own annotation, got {html:?}"
    );
    assert_tier_a(&html);
}

// ---------------------------------------------------------------------------
// Thematic breaks, code fences
// ---------------------------------------------------------------------------

#[test]
fn thematic_break_coexists_with_page_break() {
    // Two ways to separate sections: CommonMark `---` thematic break,
    // and Aozora `［＃改ページ］`. They must not interfere.
    let html = render_to_string("before\n\n---\n\n［＃改ページ］\n\nafter");
    assert!(
        html.contains("<hr"),
        "thematic break must render, got {html:?}"
    );
    assert!(
        html.contains(r#"<div class="afm-page-break"></div>"#),
        "page break must render, got {html:?}"
    );
    assert_tier_a(&html);
}

#[test]
fn fenced_code_block_preserves_aozora_markup_as_code() {
    // Aozora trigger characters inside a code fence MUST NOT be
    // interpreted. The fenced block is a raw literal per CommonMark.
    let html = render_to_string("```\n｜青梅《おうめ》\n［＃改ページ］\n```");
    assert!(
        html.contains("<pre>"),
        "code fence must render as <pre>, got {html:?}"
    );
    // The Aozora characters appear as escaped raw text inside the
    // code block — not as ruby / page-break tags.
    assert!(
        !html.contains("<ruby>"),
        "ruby tags must NOT appear inside a code fence, got {html:?}"
    );
    assert!(
        !html.contains("afm-page-break"),
        "page-break div must NOT appear inside a code fence, got {html:?}"
    );
    // Tier-A still holds — the brackets inside the code block are
    // escaped by comrak (they become `＃` + `［` etc but the canary
    // looks for `［＃` as a pair which also survives as-is inside
    // <code>).
    //
    // NOTE: the canary is intentionally broad — raw brackets in a
    // code block aren't a Tier-A violation in the spec sense (they
    // came from the author's literal markdown, not from unparsed
    // Aozora markup) but we document the current behaviour here.
    // If this assertion fails in the future, re-evaluate the Tier-A
    // rule rather than blindly "fixing" the renderer.
    let stripped = strip_annotation_wrappers(&html);
    // Brackets inside a <code> block are legitimate literal text.
    // Check only that outside <code> blocks we stay clean.
    let outside_code = scrub_code_blocks(&stripped);
    assert!(
        !outside_code.contains("［＃"),
        "bare ［＃ outside <code> block, got {html:?}"
    );
}

/// Remove every `<code>…</code>` span (including multi-line code
/// blocks) so Tier-A checks can ignore legitimate literal markup.
fn scrub_code_blocks(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find("<code") {
        out.push_str(&rest[..at]);
        let after = &rest[at..];
        let Some(end) = after.find("</code>") else {
            out.push_str(after);
            return out;
        };
        rest = &after[end + "</code>".len()..];
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// Nested containers
// ---------------------------------------------------------------------------

#[test]
fn nested_blockquote_inside_list_with_ruby_renders_all_layers() {
    let html = render_to_string("- > ｜桃《もも》引用\n- item2");
    // The nested structure: <ul><li><blockquote>…</blockquote>…</li><li>item2</li></ul>
    assert!(
        html.contains("<ul>") || html.contains("<ol>"),
        "outer list must render, got {html:?}"
    );
    assert!(
        html.contains("<blockquote>"),
        "nested blockquote must render, got {html:?}"
    );
    assert!(
        html.contains("<ruby>桃"),
        "innermost ruby must render, got {html:?}"
    );
    assert_tier_a(&html);
}

// ---------------------------------------------------------------------------
// Empty / degenerate block structures
// ---------------------------------------------------------------------------

#[test]
fn page_break_only_document_renders_single_div() {
    let html = render_to_string("［＃改ページ］");
    assert!(
        html.contains(r#"<div class="afm-page-break"></div>"#),
        "page-break-only doc must render the div, got {html:?}"
    );
    // No <p> wrapper — the lexer's blank-line padding promotes the
    // sentinel to a standalone paragraph, which post_process then
    // replaces with a pure block node.
    assert!(
        !html.contains("<p>"),
        "page-break-only doc must not emit a <p>, got {html:?}"
    );
    assert_tier_a(&html);
}

#[test]
fn empty_document_produces_empty_html() {
    let html = render_to_string("");
    assert_eq!(html, "", "empty input must yield empty HTML");
}

#[test]
fn whitespace_only_document_produces_empty_html() {
    let html = render_to_string("   \n\n   \n");
    assert_eq!(
        html.trim(),
        "",
        "whitespace-only input must yield empty HTML, got {html:?}"
    );
}
