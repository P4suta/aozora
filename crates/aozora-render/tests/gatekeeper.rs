//! 金庫番 (gatekeeper) tests for `aozora-render`.
//!
//! Pin user-visible HTML output contracts so a refactor cannot
//! silently drift the on-disk shape:
//!
//! * The five HTML-unsafe ASCII chars (`< > & " '`) each map to a
//!   specific named/numeric entity. Bumping the apostrophe form
//!   from `&#x27;` to `&#39;` (or vice versa) must be a deliberate,
//!   reviewed change — and must update BOTH renderer paths.
//! * Empty input renders to empty output (not "empty paragraph").
//! * Pure-text input round-trips byte-identical inside `<p>...</p>`.
//! * Newline semantics: single newline → `<br />`, double newline →
//!   paragraph close + reopen.
//! * `serialize` is a fixed point after one pass for canonical
//!   markup shapes (inline ruby, page break, kaeriten, gaiji).

use aozora_lex::lex_into_arena;
use aozora_render::{html, serialize};
use aozora_syntax::borrowed::Arena;

fn render_html(text: &str) -> String {
    let arena = Arena::new();
    let out = lex_into_arena(text, &arena);
    html::render_to_string(&out)
}

fn ser(text: &str) -> String {
    let arena = Arena::new();
    let out = lex_into_arena(text, &arena);
    serialize::serialize(&out)
}

#[test]
fn gatekeeper_html_entity_table_is_canonical() {
    // Pinned individually so the failure mode names which entity
    // drifted, rather than dumping a 5-char diff.
    assert!(render_html("<").contains("&lt;"), "< must escape to &lt;");
    assert!(render_html(">").contains("&gt;"), "> must escape to &gt;");
    assert!(render_html("&").contains("&amp;"), "& must escape to &amp;");
    assert!(
        render_html("\"").contains("&quot;"),
        "\" must escape to &quot;"
    );
    // Apostrophe MUST be the hex form `&#x27;`. The decimal form
    // `&#39;` is forbidden — both renderer paths agreed on hex
    // after the html.rs vs render_node.rs unification.
    let html = render_html("'");
    assert!(
        html.contains("&#x27;"),
        "apostrophe must be &#x27;, got: {html}"
    );
    assert!(!html.contains("&#39;"), "decimal &#39; leaked, got: {html}");
}

#[test]
fn gatekeeper_html_unsafe_set_is_exactly_five_chars() {
    // Every other ASCII printable must pass through as itself.
    // This is what makes the html.rs's three-needle memchr scan
    // valid — we depend on knowing every escapable byte.
    let safe_ascii: Vec<u8> = (0x20..=0x7E)
        .filter(|b| !matches!(*b, b'<' | b'>' | b'&' | b'"' | b'\''))
        .collect();
    let input = String::from_utf8(safe_ascii.clone()).unwrap();
    let html = render_html(&input);
    for &b in &safe_ascii {
        let c = b as char;
        assert!(
            html.contains(c),
            "ASCII printable {c:?} (0x{b:02X}) must pass through unescaped",
        );
    }
}

#[test]
fn gatekeeper_empty_input_renders_to_empty_string() {
    // No `<p></p>`, no whitespace — completely empty.
    assert_eq!(render_html(""), "");
}

#[test]
fn gatekeeper_pure_japanese_pass_through_unescaped() {
    // Multi-byte UTF-8 must NEVER be escaped — only the 5 ASCII
    // unsafe chars ever change form.
    let html = render_html("青空文庫の本文。");
    assert!(html.contains("青空文庫の本文。"), "got: {html}");
}

#[test]
fn gatekeeper_single_newline_becomes_br_double_closes_paragraph() {
    // The renderer's paragraph state machine has only these two
    // newline behaviours. Adding a third (e.g. CRLF handling) must
    // be a deliberate change.
    let single = render_html("a\nb");
    assert!(single.contains("a<br />\nb"), "got: {single}");
    assert!(
        !single.contains("</p>\n<p>"),
        "single \\n must NOT close para"
    );

    let double = render_html("a\n\nb");
    assert!(double.contains("<p>a</p>\n"), "got: {double}");
    assert!(double.contains("<p>b</p>\n"), "got: {double}");
}

#[test]
fn gatekeeper_serialize_is_fixed_point_for_canonical_markup() {
    // The four canonical Aozora markup shapes a real document
    // exercises must each round-trip byte-identical. If any
    // sentinel encoding/decoding drifts, serialize will not be a
    // fixed point — the second pass produces different bytes.
    for src in [
        "｜青梅《おうめ》",
        "前\n\n［＃改ページ］\n\n後",
        "学［＃二、レ点］而時習之",
        "※［＃「木＋吶のつくり」、第3水準1-85-54］",
    ] {
        let one = ser(src);
        let two = ser(&one);
        assert_eq!(one, two, "non-fixed-point on canonical input {src:?}");
    }
}

#[test]
fn gatekeeper_pua_sentinel_codepoints_in_source_dont_emit_block_tags() {
    // Phase 0 records a diagnostic but does not strip raw U+E001..
    // U+E004 from input, so they must flow through as PLAIN text
    // and never accidentally trigger structural rendering. This
    // pins "PUA collision tolerance".
    for sentinel in ["\u{E001}", "\u{E002}", "\u{E003}", "\u{E004}"] {
        let html = render_html(sentinel);
        // No `<div class="aozora-...">` should appear from a stray PUA.
        assert!(
            !html.contains(r#"<div class="aozora-page-break""#),
            "PUA {sentinel:?} accidentally produced a structural block: {html}",
        );
    }
}
