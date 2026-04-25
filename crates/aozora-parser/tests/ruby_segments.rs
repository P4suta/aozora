//! Integration tests for ruby reading `Content::Segments` lift —
//! nested gaiji / annotation inside a `《reading》` body.
//!
//! Drives the full parse + render pipeline (`aozora_parser::html::render_to_string`)
//! so regressions in any of the layers surface here:
//!
//! * **Lexer classifier** (`aozora_lexer::phase3_classify::build_content_from_body`)
//!   — folds nested `※［＃…］` and `［＃…］` into `Segment::Gaiji` /
//!   `Segment::Annotation`.
//! * **Normalize / registry** (`aozora_lexer::phase4_normalize`) — replaces
//!   the full `｜…《…》` source span with one inline PUA sentinel; the
//!   nested gaiji/annotation are carried *inside* the Ruby payload, not
//!   as sibling sentinels at the top level.
//! * **`post_process`** (`aozora_parser::post_process::splice_inline`) — the
//!   sentinel is replaced with `NodeValue::Aozora(Box::new(Ruby{…}))`;
//!   nothing special about Segments payload here, but the harness
//!   verifies the pipeline is payload-agnostic.
//! * **Renderer** (`aozora_parser::aozora::html::render_content`) — walks
//!   the Segments in order, emitting `<span class="afm-gaiji">…</span>`
//!   and `<span class="afm-annotation" hidden>…</span>` inside `<rt>`.
//!
//! Tier-A canary: no bare `［＃` may appear in the rendered HTML.
//! The renderer wraps every annotation — including the Unknown
//! fallback path exercised in
//! `render_inside_ruby_with_unrecognised_bracket_downgrades_to_annotation`
//! — in an `afm-annotation` hidden span, so the raw bytes survive
//! inside that wrapper but no bare marker leaks.

use aozora_parser::html::render_to_string;

/// Gold-standard Tier-A canary: no raw `［＃` may appear outside an
/// `afm-annotation` wrapper. Rather than try to prove the absence
/// with a regex over the full HTML (which would need to parse the
/// wrappers), we strip *all* `afm-annotation` hidden span bodies and
/// assert on what remains.
///
/// Conservative — over-strips in case an annotation body itself
/// contains `［＃`, which is exactly the escape-safe hatch we want.
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
            // Malformed — include remainder verbatim and stop.
            out.push_str(after_open);
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

/// Tier-A canary assertion. After stripping any `afm-annotation`
/// hidden-span bodies (which are allowed to carry raw `［＃`), the
/// remaining HTML must not contain the bare marker.
fn assert_tier_a(html: &str) {
    let stripped = strip_annotation_wrappers(html);
    assert!(
        !stripped.contains("［＃"),
        "bare ［＃ leaked outside afm-annotation wrapper\n  full: {html:?}\n  stripped: {stripped:?}"
    );
}

// ---------------------------------------------------------------------------
// Happy path — reading with embedded gaiji
// ---------------------------------------------------------------------------

#[test]
fn ruby_reading_with_gaiji_emits_nested_span() {
    let html = render_to_string("｜日本《に※［＃「ほ」、第3水準9-99-99］ん》");
    assert!(
        html.contains(r#"<rt>に<span class="afm-gaiji">ほ</span>ん</rt>"#),
        "expected nested gaiji span inside <rt>, got: {html:?}"
    );
    assert!(
        html.contains("<ruby>日本"),
        "base must render plain: {html:?}"
    );
    assert_tier_a(&html);
}

#[test]
fn ruby_reading_wholly_gaiji_emits_single_span_inside_rt() {
    let html = render_to_string("｜日本《※［＃「にほん」、第3水準9-99-99］》");
    assert!(
        html.contains(r#"<rt><span class="afm-gaiji">にほん</span></rt>"#),
        "whole-gaiji reading must render as a single inner span: {html:?}"
    );
    assert_tier_a(&html);
}

#[test]
fn ruby_reading_with_trailing_annotation_emits_hidden_span() {
    let html = render_to_string("｜日本《にほん［＃ママ］》");
    assert!(
        html.contains(r#"<rt>にほん<span class="afm-annotation" hidden>［＃ママ］</span></rt>"#),
        "trailing annotation must render as afm-annotation hidden span: {html:?}"
    );
    assert_tier_a(&html);
}

#[test]
fn ruby_reading_with_gaiji_and_annotation_interleave_preserves_order() {
    let html = render_to_string("｜日本《に※［＃「ほ」、第3水準9-99-99］ん［＃ママ］》");
    assert!(
        html.contains(
            r#"<rt>に<span class="afm-gaiji">ほ</span>ん<span class="afm-annotation" hidden>［＃ママ］</span></rt>"#
        ),
        "interleaved segments must render in document order: {html:?}"
    );
    assert_tier_a(&html);
}

#[test]
fn implicit_ruby_reading_with_gaiji_matches_explicit_output() {
    // Implicit form (no `｜`) takes the same body-walker path, so the
    // rendered `<rt>` body should be identical to the explicit form —
    // only the base extraction differs.
    let implicit = render_to_string("日本《に※［＃「ほ」、第3水準9-99-99］ん》");
    let explicit = render_to_string("｜日本《に※［＃「ほ」、第3水準9-99-99］ん》");
    // The `<rt>` contents must match verbatim.
    let rt_implicit = implicit
        .split_once("<rt>")
        .and_then(|(_, rest)| rest.split_once("</rt>"))
        .expect("<rt>...</rt> present in implicit");
    let rt_explicit = explicit
        .split_once("<rt>")
        .and_then(|(_, rest)| rest.split_once("</rt>"))
        .expect("<rt>...</rt> present in explicit");
    assert_eq!(rt_implicit.0, rt_explicit.0);
    assert_tier_a(&implicit);
    assert_tier_a(&explicit);
}

// ---------------------------------------------------------------------------
// Regression guards
// ---------------------------------------------------------------------------

#[test]
fn plain_reading_still_renders_unchanged_from_pre_f1_shape() {
    // Fast-path regression: plain ruby must keep rendering on the
    // `Content::Plain` path. If it's ever accidentally lifted to
    // `Segments([Text])` the render is identical but the allocation
    // is spurious — checked indirectly through the other tests in
    // this file; here we just pin the rendered bytes.
    let html = render_to_string("｜青梅《おうめ》");
    assert!(html.contains("<ruby>青梅<rp>(</rp><rt>おうめ</rt><rp>)</rp></ruby>"));
    assert_tier_a(&html);
}

#[test]
fn html_escape_of_ruby_reading_segments_still_applies() {
    // HTML-escape invariant must propagate into `<rt>` text segments.
    // Construct a reading whose text contains all five OWASP HTML
    // escape targets around a gaiji marker — the gaiji description
    // itself is also escaped by `render_gaiji`.
    // Mencode `U+ZZZZ` intentionally does NOT parse as hex, so the
    // `parse_u_plus` path misses and the renderer falls back to the
    // description bytes — which is where the HTML-escape invariant
    // must show its work.
    let html = render_to_string("｜x《<&>\"'※［＃「a<b」、U+ZZZZ］<&>'\"》");
    // The five escapes must appear in the Text segments of the `<rt>`.
    let (_, tail) = html.split_once("<rt>").expect("<rt> present");
    let (rt_body, _) = tail.split_once("</rt>").expect("</rt> present");
    assert!(
        rt_body.contains("&lt;"),
        "<rt> must escape `<`: {rt_body:?}"
    );
    assert!(
        rt_body.contains("&amp;"),
        "<rt> must escape `&`: {rt_body:?}"
    );
    assert!(
        rt_body.contains("&gt;"),
        "<rt> must escape `>`: {rt_body:?}"
    );
    assert!(
        rt_body.contains("&quot;"),
        "<rt> must escape `\"`: {rt_body:?}"
    );
    assert!(
        rt_body.contains("&#x27;"),
        "<rt> must escape `'`: {rt_body:?}"
    );
    // The gaiji description `a<b` must also escape its `<`.
    assert!(
        rt_body.contains(r#"<span class="afm-gaiji">a&lt;b</span>"#),
        "gaiji description must escape its `<`: {rt_body:?}"
    );
    // XSS canary.
    assert!(!html.contains("<script"), "no raw <script> tag: {html:?}");
    assert_tier_a(&html);
}

#[test]
fn render_inside_ruby_with_unrecognised_bracket_downgrades_to_annotation() {
    // `［＃改ページ］` is a block-leaf in isolation; inside a ruby
    // reading it's meaningless, so the lexer must downgrade it to
    // `Annotation{Unknown}` and the renderer must wrap it in an
    // `afm-annotation` hidden span — no raw ［＃ leaks out.
    let html = render_to_string("｜日本《にほん［＃改ページ］》");
    assert!(
        html.contains(r#"<span class="afm-annotation" hidden>［＃改ページ］</span>"#),
        "nested block-leaf must downgrade to Annotation{{Unknown}}: {html:?}"
    );
    // No standalone afm-page-break div inside the ruby reading.
    let (before_ruby, after_open_rt) = html.split_once("<rt>").expect("ruby `<rt>` present");
    let (rt_body, _rest) = after_open_rt.split_once("</rt>").expect("</rt> present");
    assert!(
        !rt_body.contains("afm-page-break"),
        "page-break div must not appear inside <rt>: {rt_body:?}"
    );
    assert!(!before_ruby.contains("afm-page-break"));
    assert_tier_a(&html);
}

#[test]
fn ruby_with_nested_gaiji_produces_exactly_one_top_level_aozora_sentinel() {
    // Smoke test on the outer-span consolidation: a ruby with
    // embedded gaiji must produce ONE `<ruby>` in the rendered HTML
    // (not two, which would indicate the inner gaiji leaked to a
    // sibling span at the top level).
    let html = render_to_string("前｜日本《に※［＃「ほ」、第3水準9-99-99］ん》後");
    let ruby_opens = html.matches("<ruby>").count();
    assert_eq!(
        ruby_opens, 1,
        "exactly one <ruby> element expected, got {ruby_opens}: {html:?}"
    );
    let gaiji_spans = html.matches(r#"<span class="afm-gaiji">"#).count();
    assert_eq!(
        gaiji_spans, 1,
        "exactly one gaiji span expected (nested inside ruby), got {gaiji_spans}: {html:?}"
    );
    // And it must be INSIDE the <rt>, not a sibling.
    let (_, rt_and_after) = html.split_once("<rt>").expect("<rt> present");
    let (rt_body, _) = rt_and_after.split_once("</rt>").expect("</rt> present");
    assert!(
        rt_body.contains(r#"<span class="afm-gaiji">"#),
        "gaiji span must be inside <rt>, not sibling: {html:?}"
    );
    assert_tier_a(&html);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[test]
fn strip_annotation_wrappers_removes_only_annotation_body() {
    // Self-test for the test-harness stripper — a regression here
    // would silently weaken the Tier-A canary on the real tests.
    let input = "前<span class=\"afm-annotation\" hidden>［＃改ページ］</span>後";
    assert_eq!(strip_annotation_wrappers(input), "前後");
    // Unterminated hidden span — stripper should gracefully include
    // the tail without panicking.
    let broken = "前<span class=\"afm-annotation\" hidden>［＃改ページ";
    assert_eq!(strip_annotation_wrappers(broken), "前［＃改ページ");
    // No annotation — identity.
    assert_eq!(strip_annotation_wrappers("<p>plain</p>"), "<p>plain</p>");
}
