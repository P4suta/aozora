//! Hand-written Aozora-annotation conformance.
//!
//! Iterates every `spec/aozora/cases/*.json` fixture file; each contains an
//! array of `{name, markdown, html}` entries. Each case is rendered through
//! `Options::afm_default()` and its HTML output compared byte-for-byte with
//! the expected string.
//!
//! Adding a new annotation kind is a pattern:
//! 1. drop a new `spec/aozora/cases/<kind>.json` with the expected cases,
//! 2. extend `aozora::annotation::classify` to promote the body,
//! 3. run `just spec-aozora`.
//!
//! Fixtures live under `spec/aozora/cases/` (distinct from
//! `spec/aozora/fixtures/` which holds full-work golden texts like 『罪と罰』).

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Case {
    name: String,
    markdown: String,
    html: String,
}

/// Load every `spec/aozora/cases/*.json` file the build-time `include_bytes!`
/// macro can see. The list is maintained by hand below so adding a new
/// fixture file forces a visible commit here (makes the spec surface
/// explicit).
fn all_cases() -> Vec<(&'static str, Vec<Case>)> {
    const LAYOUT_PAGE_BREAK: &str =
        include_str!("../../../spec/aozora/cases/layout-page-break.json");
    const EMPHASIS_BOUTEN: &str = include_str!("../../../spec/aozora/cases/emphasis-bouten.json");
    const ETC_TCY: &str = include_str!("../../../spec/aozora/cases/etc-tate-chu-yoko.json");
    vec![
        (
            "layout-page-break.json",
            serde_json::from_str(LAYOUT_PAGE_BREAK).expect("layout-page-break.json parses"),
        ),
        (
            "emphasis-bouten.json",
            serde_json::from_str(EMPHASIS_BOUTEN).expect("emphasis-bouten.json parses"),
        ),
        (
            "etc-tate-chu-yoko.json",
            serde_json::from_str(ETC_TCY).expect("etc-tate-chu-yoko.json parses"),
        ),
    ]
}

#[test]
fn aozora_notation_fixtures() {
    let mut failures: Vec<String> = Vec::new();
    let mut total = 0usize;

    for (file, cases) in all_cases() {
        for case in &cases {
            total += 1;
            let actual = afm_parser::html::render_to_string(&case.markdown);
            if actual != case.html {
                failures.push(format!(
                    "{file}::{name}\n  markdown: {md:?}\n  expected: {exp:?}\n  actual:   {act:?}",
                    name = case.name,
                    md = case.markdown,
                    exp = case.html,
                    act = actual,
                ));
            }
        }
    }

    assert!(total > 0, "no aozora fixtures loaded; did the paths drift?");
    assert!(
        failures.is_empty(),
        "{n} aozora notation fixture(s) failed:\n\n{joined}",
        n = failures.len(),
        joined = failures.join("\n\n"),
    );
}
