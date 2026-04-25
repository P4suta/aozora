//! Byte-exact regression pins for the `aozora_parser::html` block
//! walker.
//!
//! Each fixture in this file pins a specific output shape that a
//! prior bug produced incorrectly, so the next refactor of the
//! walker either preserves behaviour or surfaces the regression as
//! a single-line `assert_eq!` failure.
//!
//! When the renderer's output convention legitimately evolves (e.g.
//! a future heading promotion lands in pure-aozora HTML), update
//! the fixture *and* extend the commit message with a `BREAKING:`
//! line so downstream `aozora-tools` knows to bump.

use aozora_parser::html::render_to_string;

// ---------------------------------------------------------------------
// Trailing-newline / single-newline boundary
// ---------------------------------------------------------------------

/// 2026-04-25 — early walker emitted `<br />` for every `\n`,
/// including the trailing newline of the buffer. Spec fixtures
/// (`emphasis-bouten.json`) expect a clean `</p>\n` without a
/// dangling hardbreak before the close tag.
#[test]
fn trailing_newline_does_not_dangle_a_br() {
    let html = render_to_string("責［＃「責」にばつ傍点］めて\n");
    let expected =
        "<p>責<em class=\"afm-bouten afm-bouten-cross afm-bouten-right\">責</em>めて</p>\n";
    assert_eq!(html, expected);
}

#[test]
fn trailing_double_newline_does_not_dangle_a_br_either() {
    let html = render_to_string("Hello\n\n");
    assert_eq!(html, "<p>Hello</p>\n");
}

/// Hard-break in the middle of a paragraph is the *correct* behaviour
/// for a single `\n` between content; the trailing-newline fix above
/// must not regress this.
#[test]
fn mid_paragraph_single_newline_emits_hardbreak_not_paragraph_split() {
    let html = render_to_string("verse one\nverse two");
    assert_eq!(html, "<p>verse one<br />\nverse two</p>\n");
}

// ---------------------------------------------------------------------
// Block-leaf separator placement
// ---------------------------------------------------------------------

/// 2026-04-25 — early walker unconditionally pushed `\n` after every
/// block element, breaking the standalone-leaf shape. The fix
/// (`pending_block_separator` flag) emits the separator only when
/// more content follows. Standalone leaf must end at `</div>` with
/// no trailing `\n`.
#[test]
fn standalone_page_break_has_no_trailing_newline() {
    let html = render_to_string("［＃改ページ］\n");
    assert_eq!(html, r#"<div class="afm-page-break"></div>"#);
}

#[test]
fn standalone_section_break_choho_has_no_trailing_newline() {
    let html = render_to_string("［＃改丁］\n");
    assert_eq!(
        html,
        r#"<div class="afm-section-break afm-section-break-choho"></div>"#
    );
}

/// 2026-04-25 — companion case to the standalone fix: when a block
/// leaf is sandwiched between paragraphs, the separator `\n` must
/// fire so the next `<p>` starts on a fresh line.
#[test]
fn block_leaf_between_paragraphs_emits_separator_newline() {
    let html = render_to_string("前［＃改ページ］後\n");
    assert_eq!(
        html,
        "<p>前</p>\n<div class=\"afm-page-break\"></div>\n<p>後</p>\n"
    );
}

#[test]
fn two_consecutive_block_leafs_separator_fires_between_them() {
    let html = render_to_string("［＃改ページ］\n\n［＃改丁］\n");
    // The `pending_block_separator` flag must fire on the
    // second leaf so its open tag lands on a fresh line, not
    // adjacent to the first leaf's close tag.
    assert_eq!(
        html,
        "<div class=\"afm-page-break\"></div>\n<div class=\"afm-section-break afm-section-break-choho\"></div>"
    );
}

// ---------------------------------------------------------------------
// Container nesting
// ---------------------------------------------------------------------

/// 2026-04-25 — empty container (immediate open + close) must still
/// produce a balanced `<div>...</div>` shell. Pinned because an
/// early walker that conditionally suppressed the `</div>` on empty
/// bodies broke the tag-balance invariant.
#[test]
fn empty_indent_container_still_emits_paired_div() {
    let html = render_to_string("［＃ここから2字下げ］\n\n［＃ここで字下げ終わり］\n");
    assert!(
        html.starts_with("<div class=\"afm-container afm-container-indent"),
        "container open absent: {html:?}"
    );
    assert!(html.contains("</div>"), "container close absent: {html:?}");
    // No dangling paragraph inside the shell.
    assert!(
        !html.contains("<p></p>"),
        "phantom empty <p></p> in empty container: {html:?}"
    );
}

#[test]
fn indent_container_with_paragraph_body_pins_layout() {
    let html = render_to_string("［＃ここから2字下げ］\n\n本文\n\n［＃ここで字下げ終わり］\n");
    let expected = concat!(
        "<div class=\"afm-container afm-container-indent afm-container-indent-2\" data-amount=\"2\">\n",
        "<p>本文</p>\n",
        "</div>",
    );
    assert_eq!(html, expected);
}

// ---------------------------------------------------------------------
// HTML escape
// ---------------------------------------------------------------------

#[test]
fn the_five_html_entities_round_trip_through_escape() {
    // Pin the exact entity strings used by `escape_text_chunk`.
    let html = render_to_string(r#"<>&"'"#);
    assert_eq!(html, "<p>&lt;&gt;&amp;&quot;&#39;</p>\n");
}

#[test]
fn pre_escaped_amp_in_source_double_escapes() {
    // A regression that interpreted `&` as already-escaped would
    // emit `&amp;` raw; the renderer treats source as plain text and
    // therefore double-escapes.
    let html = render_to_string("&amp;");
    assert_eq!(html, "<p>&amp;amp;</p>\n");
}

#[test]
fn xss_marker_in_source_is_neutralised() {
    let html = render_to_string("<script>alert(1)</script>");
    assert!(
        !html.contains("<script"),
        "live <script tag survived: {html:?}"
    );
    assert!(html.contains("&lt;script"));
    assert!(html.contains("alert(1)"));
}

// ---------------------------------------------------------------------
// Tier-A floor — bare ［＃ outside a wrapper is a hard-block invariant
// for any input the lexer accepted without complaint.
// ---------------------------------------------------------------------

#[test]
fn unknown_bracket_annotation_wrapped_in_afm_annotation_span() {
    let html = render_to_string("前［＃ほげふが］後");
    assert!(
        html.contains(r#"<span class="afm-annotation" hidden>"#),
        "annotation wrapper missing: {html:?}"
    );
    assert!(html.contains("[#ほげふが]") || html.contains("［＃ほげふが］"));
}
