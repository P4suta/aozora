//! Isolation test for forward-reference bouten handling.
//!
//! Tier A on the full 罪と罰 fixture was leaking ［＃ at
//! `可哀想［＃「可哀想」に傍点］` — this integration test pins that exact pattern
//! so a regression surfaces immediately instead of only when the full novel is
//! exercised.

#[test]
fn forward_reference_bouten_is_wrapped_in_annotation_html() {
    let src = "可哀想［＃「可哀想」に傍点］という気";
    let html = afm_parser::html::render_to_string(src);

    // The bracket-annotation body should survive only inside the afm-annotation
    // wrapper. When the wrapper is stripped, no bare ［＃ may remain.
    assert!(
        html.contains(r#"<span class="afm-annotation" hidden>［＃"#),
        "missing afm-annotation wrapper around the bouten annotation: {html}"
    );

    let stripped = strip_annotation_wrappers(&html);
    assert!(
        !stripped.contains("［＃"),
        "［＃ leaked outside the wrapper.\n  html: {html}\n  stripped: {stripped}"
    );
}

#[test]
fn forward_reference_bouten_survives_long_paragraph_context() {
    // Exact snippet around the leak 罪と罰 Tier A flags. The critical character
    // class in the prefix is the opening curly quote 「 U+300C, which shares a
    // UTF-8 lead byte (0xE3) with 《 — if `find_aozora_trigger_offset` ever
    // matched 「, the scanner would stop inside an unrelated opening-quote span
    // and the subsequent ［＃ would never get classified.
    let src = "「そう、妹さんの心の中に可哀想［＃「可哀想」に傍点］という気が起こる";
    let html = afm_parser::html::render_to_string(src);
    let stripped = strip_annotation_wrappers(&html);
    assert!(
        !stripped.contains("［＃"),
        "［＃ leaked:\n  html: {html}\n  stripped: {stripped}"
    );
}

#[test]
fn curly_quote_followed_by_bracket_annotation() {
    // Minimal repro: the 「X」 quote form appears in many contexts; the adapter
    // must not confuse 「 with a trigger character.
    let src = "「X」［＃「Y」に傍点］";
    let html = afm_parser::html::render_to_string(src);
    let stripped = strip_annotation_wrappers(&html);
    assert!(!stripped.contains("［＃"), "［＃ leaked: {html}");
}

fn strip_annotation_wrappers(html: &str) -> String {
    let opener = r#"<span class="afm-annotation" hidden>"#;
    let closer = "</span>";
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find(opener) {
        out.push_str(&rest[..at]);
        let after_open = &rest[at + opener.len()..];
        if let Some(close_at) = after_open.find(closer) {
            rest = &after_open[close_at + closer.len()..];
        } else {
            out.push_str(rest);
            return out;
        }
    }
    out.push_str(rest);
    out
}
