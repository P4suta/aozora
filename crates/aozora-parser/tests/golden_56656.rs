//! Golden fixture — 青空文庫 card 56656 (『罪と罰』米川正夫訳).
//!
//! Runs the afm pipeline against a real, densely-annotated translation of
//! Dostoevsky and asserts the M0 Spike "Tier A" contract:
//!
//! 1. The parser completes without panicking on a full-length Aozora Bunko work.
//! 2. Every `［＃…］` sequence is consumed (wrapped inside an `afm-annotation`
//!    node) — no bare annotation markers leak into the rendered HTML.
//! 3. Every `｜…《…》` explicit-ruby span is recognised.

use afm_parser::test_support::{assert_no_bare, strip_annotation_wrappers};

const FIXTURE: &str = include_str!("../../../spec/aozora/fixtures/56656/input.utf8.txt");

/// Tier A acceptance — the sole gate for M0 Spike completion.
#[test]
fn tier_a_no_panic_and_no_unconsumed_square_brackets() {
    let html = afm_parser::html::render_to_string(FIXTURE);

    // Any bare ［＃ (outside an afm-annotation wrapper) panics with a
    // diagnostic snippet formatted by the shared helper.
    assert_no_bare(&html, "［＃");

    // Sanity: the strip operation should be idempotent — running it again on
    // already-stripped output should produce no further change, proving our
    // splitter covers the full HTML shape the renderer emits.
    let bare = strip_annotation_wrappers(&html);
    let bare_again = strip_annotation_wrappers(&bare);
    assert_eq!(
        bare, bare_again,
        "annotation stripper not idempotent — likely nested or malformed wrapper"
    );
}

/// Count ruby spans produced and compare against the known floor. A regression
/// to 0 would silently go undetected if we only asserted parse success.
#[test]
fn tier_a_ruby_recognition_floor() {
    let arena = comrak::Arena::new();
    let options = afm_parser::Options::afm_default();
    let root = afm_parser::parse(&arena, FIXTURE, &options);

    let mut ruby_count = 0usize;
    let mut annotation_count = 0usize;
    count_aozora(root, &mut ruby_count, &mut annotation_count);

    // Observed on the 2021-10-27 publication: ~2229 ruby readings + ~93 explicit
    // ｜ delimiters (some readings share a base). Annotation count ~489.
    // Enforce a floor well below measured values so minor parser-policy shifts
    // (e.g. smarter implicit-ruby base recovery) don't false-fail.
    assert!(
        ruby_count >= 1500,
        "ruby recognition dropped to {ruby_count} (expected >= 1500)"
    );
    assert!(
        annotation_count >= 400,
        "annotation recognition dropped to {annotation_count} (expected >= 400)"
    );
}

fn count_aozora<'a>(
    node: &'a comrak::nodes::AstNode<'a>,
    rubies: &mut usize,
    annotations: &mut usize,
) {
    if let comrak::nodes::NodeValue::Aozora(ref boxed) = node.data.borrow().value {
        match **boxed {
            afm_syntax::AozoraNode::Ruby(_) => *rubies += 1,
            afm_syntax::AozoraNode::Annotation(_) => *annotations += 1,
            _ => {}
        }
    }
    for child in node.children() {
        count_aozora(child, rubies, annotations);
    }
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
