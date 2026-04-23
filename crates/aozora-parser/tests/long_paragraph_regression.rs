//! Reproduce the Tier A leak on an isolated long paragraph so we don't need
//! 2 MB of context to debug each iteration.

use afm_parser::html::render_to_string;
use afm_parser::test_support::assert_no_bare;

const FIXTURE: &str = include_str!("../../../spec/aozora/fixtures/56656/input.utf8.txt");

#[test]
fn long_paragraph_consumes_all_bracket_annotations() {
    // Line 3713 of 罪と罰 is a single ~30KB paragraph containing 11 of the
    // Tier A leak sites. Isolating it keeps diagnostic output manageable.
    let line = FIXTURE
        .lines()
        .find(|l| l.contains("可哀想［＃「可哀想」に傍点］"))
        .expect("target paragraph present in fixture");
    assert!(
        line.len() > 10_000,
        "expected long paragraph, got {}",
        line.len()
    );

    let html = render_to_string(line);
    // Delegates the strip + context-formatting to the shared helper so any
    // leak panics with a diagnostic snippet ready for use.
    assert_no_bare(&html, "［＃");
}
