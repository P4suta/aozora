//! Golden fixture — 青空文庫 card 56656 (『罪と罰』米川正夫訳).
//!
//! Runs the afm pipeline against a real annotated work and asserts the M0 Spike
//! "Tier A" promise: no parser panic, and no leftover `［＃` sequences after parsing
//! (i.e. every Aozora annotation was recognised and consumed into an AST node).
//!
//! At M0 Spike entry, the parser isn't wired into the comrak fork yet; the test is
//! `#[ignore]` with a clear reason so `just spec-golden-56656` reports "ignored" rather
//! than silently passing. Removing the `#[ignore]` line is the Definition of Done for
//! M0 Spike.

const FIXTURE: &str = include_str!("../../../spec/aozora/fixtures/56656/input.utf8.txt");

#[test]
#[ignore = "M0 Spike in progress — comrak fork not yet wired; Tier A harness lives here"]
fn tier_a_no_panic_and_no_unconsumed_square_brackets() {
    // When the parser is online, uncomment:
    // let html = afm_parser::html::render_to_string(FIXTURE);
    // assert!(!html.contains("［＃"), "unconsumed annotation markers remain in output");

    // Until then, at least verify the fixture loads and is non-trivial:
    let _ = FIXTURE;
    assert!(
        FIXTURE.contains("｜"),
        "fixture should contain explicit ruby delimiters"
    );
    assert!(
        FIXTURE.contains("《"),
        "fixture should contain ruby readings"
    );
    assert!(
        FIXTURE.contains("［＃"),
        "fixture should contain Aozora block annotations"
    );
}

/// Census the annotation-shaped sequences in the raw source. Serves as a canary on the
/// fixture itself: if these counts drift, the vendored file was truncated or
/// re-encoded badly. Values are measured from the 2021-10-27 publication by 青空文庫.
#[test]
fn fixture_annotation_census_matches_publication() {
    let ruby_opens = FIXTURE.matches('《').count();
    let ruby_closes = FIXTURE.matches('》').count();
    let bar_delimiter = FIXTURE.matches('｜').count();
    let block_annotation = FIXTURE.matches("［＃").count();
    let gaiji_marker = FIXTURE.matches("※［＃").count();

    assert_eq!(ruby_opens, 2229, "《 count moved from 2229");
    assert_eq!(ruby_closes, 2229, "》 count moved from 2229");
    assert_eq!(bar_delimiter, 93, "｜ count moved from 93");
    assert_eq!(block_annotation, 489, "［＃ count moved from 489");
    assert_eq!(gaiji_marker, 3, "※［＃ (gaiji) count moved from 3");
    assert_eq!(
        ruby_opens, ruby_closes,
        "ruby opens and closes must be balanced"
    );
}
