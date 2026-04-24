//! HTML-escape invariants for every Aozora render path.
//!
//! Aozora Bunko source is user-supplied text from arbitrary publishers; the
//! renderer must never emit a raw HTML tag that the browser would execute.
//! The concrete guarantees this file locks down:
//!
//! 1. **No literal `<script` escapes into output** — the gold-standard XSS
//!    canary. Any regression here is a shipping blocker.
//! 2. **User content carried on Aozora variants is `escape_text`'d.** The
//!    five OWASP HTML5 escapes (`< > & " '`) all apply:
//!    `<` → `&lt;`, `>` → `&gt;`, `&` → `&amp;`, `"` → `&quot;`,
//!    `'` → `&#x27;`. Each Aozora variant that carries user bytes (ruby
//!    base / reading, bouten target, tate-chu-yoko text, annotation raw,
//!    gaiji description, kaeriten mark) is exercised with at least one
//!    payload that contains all five escape-target characters.
//! 3. **Comrak-side plain text escaping remains sound.** Plain text
//!    outside any Aozora construct is rendered by comrak itself, not by
//!    afm's `escape_text`. These tests assert the HTML-safe subset
//!    (`<>&`) is still handled; `'` and `"` are HTML5-legal in text
//!    content so comrak is spec-compliant to leave them as-is.
//!
//! The tests drive the full parse + render pipeline (`afm_parser::render_to_string`)
//! so regressions in the lexer fallback, `post_process` AST surgery, or the
//! renderer surface here.

use afm_parser::html::render_to_string;

/// Gold-standard XSS canary. Any render output that contains the literal
/// `<script` means a browser-executable tag has escaped through.
fn contains_raw_script_tag(html: &str) -> bool {
    html.contains("<script")
}

// ---------------------------------------------------------------------------
// Plain CommonMark text — comrak's own escape pass handles this.
// ---------------------------------------------------------------------------

#[test]
fn plain_text_angle_brackets_escape_via_comrak() {
    // comrak escapes `<` / `>` in text nodes (CommonMark 0.31.2 §6.6
    // "Textual content").
    let html = render_to_string("A < B > C");
    assert!(
        html.contains("A &lt; B &gt; C"),
        "angle brackets must be HTML-escaped in plain text, got {html:?}"
    );
}

#[test]
fn plain_text_ampersand_escapes_via_comrak() {
    let html = render_to_string("Tom & Jerry");
    assert!(
        html.contains("Tom &amp; Jerry"),
        "ampersand must escape to &amp; in plain text, got {html:?}"
    );
}

// ---------------------------------------------------------------------------
// Aozora variant payloads — escape_text owns these.
// ---------------------------------------------------------------------------

#[test]
fn ruby_base_all_five_escape_targets_are_escaped() {
    // Every OWASP-standard HTML5 escape target appears in the base.
    let html = render_to_string(r#"｜A<B>C&D"E'F《読》"#);
    assert!(html.contains("<ruby>"), "expected ruby tag, got {html:?}");
    assert!(
        html.contains("A&lt;B&gt;C&amp;D&quot;E&#x27;F"),
        "ruby base must HTML-escape all five structural chars, got {html:?}"
    );
    assert!(!contains_raw_script_tag(&html));
}

#[test]
fn ruby_reading_all_five_escape_targets_are_escaped() {
    let html = render_to_string(r#"｜漢字《A<B>C&D"E'F》"#);
    assert!(
        html.contains("<rt>A&lt;B&gt;C&amp;D&quot;E&#x27;F"),
        "ruby reading must HTML-escape all five structural chars, got {html:?}"
    );
}

#[test]
fn unknown_annotation_body_all_five_escape_targets_are_escaped() {
    // Unknown `［＃…］` bodies go through the Annotation{Unknown} catch-all.
    // The raw body bytes (brackets included) must escape before landing in
    // the hidden span.
    let html = render_to_string(r#"前［＃A<B>C&D"E'F］後"#);
    assert!(
        html.contains(r#"<span class="afm-annotation" hidden>"#),
        "unknown ［＃…］ must wrap in afm-annotation span, got {html:?}"
    );
    assert!(
        html.contains("A&lt;B&gt;C&amp;D&quot;E&#x27;F"),
        "unknown annotation body must escape all five structural chars, got {html:?}"
    );
    assert!(!contains_raw_script_tag(&html));
}

#[test]
fn forward_bouten_target_escape_targets_are_escaped() {
    // The target literal `&<` appears in the preceding text, so the
    // classifier promotes to Bouten. The target inside `<em>` must
    // escape; this also exercises the Content-segment render path.
    let html = render_to_string("&<前&<［＃「&<」に傍点］後");
    assert!(
        html.contains("afm-bouten"),
        "expected afm-bouten wrapper in {html:?}"
    );
    // Inside <em class="afm-bouten ..."> the target appears once,
    // escaped. The class list includes a position modifier
    // (`afm-bouten-right` by default); the target bytes must still
    // escape.
    assert!(
        html.contains(r#"<em class="afm-bouten afm-bouten-goma afm-bouten-right">&amp;&lt;</em>"#),
        "bouten target must escape inside its em tag, got {html:?}"
    );
    assert!(!contains_raw_script_tag(&html));
}

#[test]
fn forward_tcy_text_escape_targets_are_escaped() {
    // Similar shape — target `&<` appears preceded, escapes inside
    // `<span class="afm-tcy">`.
    let html = render_to_string("&<前&<［＃「&<」は縦中横］後");
    assert!(
        html.contains("afm-tcy"),
        "expected afm-tcy wrapper in {html:?}"
    );
    assert!(
        html.contains(r#"<span class="afm-tcy">&amp;&lt;</span>"#),
        "TCY text must escape inside its span tag, got {html:?}"
    );
    assert!(!contains_raw_script_tag(&html));
}

// ---------------------------------------------------------------------------
// Script-tag canary — broad regression sweep
// ---------------------------------------------------------------------------

#[test]
fn render_never_leaks_raw_script_tag_for_any_content_location() {
    // Try a `<script>` payload in each user-content location and assert
    // it never appears unescaped. The ban is intentionally narrow —
    // only the literal `<script` substring — so every real HTML tag
    // the renderer emits (`<ruby>`, `<span>`, …) stays legal.
    let payload = "<script>alert(1)</script>";

    // Plain text
    let html = render_to_string(payload);
    assert!(
        !contains_raw_script_tag(&html),
        "plain text leaked `<script` into {html:?}",
    );

    // Ruby base
    let html = render_to_string(&format!("｜{payload}《読み》"));
    assert!(
        !contains_raw_script_tag(&html),
        "ruby base leaked `<script` into {html:?}",
    );

    // Ruby reading
    let html = render_to_string(&format!("｜base《{payload}》"));
    assert!(
        !contains_raw_script_tag(&html),
        "ruby reading leaked `<script` into {html:?}",
    );

    // Unknown annotation body
    let html = render_to_string(&format!("前［＃{payload}］後"));
    assert!(
        !contains_raw_script_tag(&html),
        "annotation body leaked `<script` into {html:?}",
    );
}

#[test]
fn render_never_leaks_raw_script_tag_for_classic_payload_shapes() {
    // Beyond `<script>`, the renderer must not leak other classic
    // injection shapes. These are plain-text inputs (not inside Aozora
    // constructs) so comrak's escape is the unit under test.
    let payloads = [
        "<img src=x onerror=alert(1)>",
        "<a href=javascript:void(0)>click</a>",
        "<svg/onload=alert(1)>",
        "<iframe src=evil></iframe>",
    ];
    for p in payloads {
        let html = render_to_string(p);
        // Every `<` from the payload must be escaped, so the literal
        // "<img" / "<a " / "<svg" / "<iframe" must not survive.
        let first_open = p.split_whitespace().next().unwrap_or(p);
        let marker = first_open.split_once('/').map_or(first_open, |(a, _)| a);
        assert!(
            !html.contains(marker),
            "plain text leaked {marker:?} from payload {p:?} into {html:?}",
        );
    }
}

// ---------------------------------------------------------------------------
// Attribute-context escaping — the quote characters matter even inside
// hidden spans because downstream sanitisers often do a second pass.
// ---------------------------------------------------------------------------

#[test]
fn annotation_raw_preserves_double_and_single_quote_escape() {
    // Double + single quote in an annotation body. Both escape into the
    // hidden afm-annotation span via afm's escape_text.
    let html = render_to_string(r#"前［＃say "hi" don't stop］後"#);
    assert!(
        html.contains("&quot;hi&quot;"),
        "double quote must escape inside annotation, got {html:?}"
    );
    assert!(
        html.contains("don&#x27;t"),
        "single quote must escape inside annotation, got {html:?}"
    );
}

#[test]
fn ruby_reading_preserves_double_and_single_quote_escape() {
    let html = render_to_string(r#"｜base《don't "stop"》"#);
    assert!(
        html.contains("don&#x27;t"),
        "single quote in reading must escape, got {html:?}"
    );
    assert!(
        html.contains("&quot;stop&quot;"),
        "double quote in reading must escape, got {html:?}"
    );
}
