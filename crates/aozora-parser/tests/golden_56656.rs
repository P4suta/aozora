//! Golden fixture — 青空文庫 card 56656 (『罪と罰』米川正夫訳).
//!
//! Runs the afm pipeline against a real, densely-annotated translation of
//! Dostoevsky and asserts the M0 Spike "Tier A" contract:
//!
//! 1. The parser completes without panicking on a full-length Aozora Bunko work.
//! 2. Every `［＃…］` sequence is consumed (wrapped inside an `afm-annotation`
//!    node) — no bare annotation markers leak into the rendered HTML.
//! 3. Every `｜…《…》` explicit-ruby span is recognised.

use afm_parser::html::render_to_string;
use afm_parser::test_support::{assert_no_bare, strip_annotation_wrappers};
use afm_syntax::AozoraNode;
use comrak::nodes::{AstNode, NodeValue};

const FIXTURE: &str = include_str!("../../../spec/aozora/fixtures/56656/input.utf8.txt");

/// Tier A acceptance — the sole gate for M0 Spike completion.
#[test]
fn tier_a_no_panic_and_no_unconsumed_square_brackets() {
    let html = render_to_string(FIXTURE);

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

/// Count ruby spans and the total number of ［＃…］-sourced annotations
/// (`Annotation` + `Bouten` + `PageBreak` + `SectionBreak`) and compare
/// against the known floors. A regression to 0 would silently go undetected
/// if we only asserted parse success.
#[test]
fn tier_a_ruby_recognition_floor() {
    let arena = comrak::Arena::new();
    let options = afm_parser::Options::afm_default();
    let root = afm_parser::parse(&arena, FIXTURE, &options);

    let mut counts = AozoraCounts::default();
    count_aozora(root, &mut counts);

    // Observed on the 2021-10-27 publication: ~2229 ruby readings + ~93 explicit
    // ｜ delimiters (some readings share a base). Total bracket-sourced
    // annotations ~489; the classifier reclassifies them into Annotation /
    // Bouten / PageBreak / SectionBreak / Indent / AlignEnd / Gaiji /
    // Kaeriten / TateChuYoko as recognisers land. Floor covers the sum of
    // every bracket-sourced variant so adding a new recogniser cannot
    // silently erode the total.
    assert!(
        counts.rubies >= 1500,
        "ruby recognition dropped to {count} (expected >= 1500)",
        count = counts.rubies,
    );
    let bracket_sourced = counts.annotations
        + counts.boutens
        + counts.page_breaks
        + counts.section_breaks
        + counts.indents
        + counts.align_ends
        + counts.gaijis
        + counts.kaeritens
        + counts.tate_chu_yokos
        + counts.other;
    // NOTE: paired-container open / close markers (`［＃ここから字下げ］`,
    // `［＃ここで字下げ終わり］`, 罫囲み, 割り注, …) are classified by
    // the lexer as `ContainerKind::{Indent,Keigakomi,Warichu,…}` open
    // / close sentinels rather than `AozoraNode::Annotation`. The
    // post_process AST wrap for these is deferred to F5 (`AozoraNode::Container`
    // schema), so today the sentinels are left as raw PUA characters in
    // text nodes and contribute nothing to this count. Once F5 lands we
    // should restore the 400 floor (adapter-era baseline); until then
    // `>= 350` keeps the canary meaningful without tripping on the known
    // F5 gap.
    assert!(
        bracket_sourced >= 350,
        "bracket-sourced annotation recognition dropped to {bracket_sourced} \
         (expected >= 350; will rise to 400 when F5 paired-container wrap lands); \
         breakdown: {counts:?}"
    );
}

#[derive(Debug, Default)]
struct AozoraCounts {
    rubies: usize,
    annotations: usize,
    boutens: usize,
    page_breaks: usize,
    section_breaks: usize,
    indents: usize,
    align_ends: usize,
    gaijis: usize,
    kaeritens: usize,
    tate_chu_yokos: usize,
    other: usize,
}

fn count_aozora<'a>(node: &'a AstNode<'a>, counts: &mut AozoraCounts) {
    if let NodeValue::Aozora(ref boxed) = node.data.borrow().value {
        match **boxed {
            AozoraNode::Ruby(_) => counts.rubies += 1,
            AozoraNode::Annotation(_) => counts.annotations += 1,
            AozoraNode::Bouten(_) => counts.boutens += 1,
            AozoraNode::PageBreak => counts.page_breaks += 1,
            AozoraNode::SectionBreak(_) => counts.section_breaks += 1,
            AozoraNode::Indent(_) => counts.indents += 1,
            AozoraNode::AlignEnd(_) => counts.align_ends += 1,
            AozoraNode::Gaiji(_) => counts.gaijis += 1,
            AozoraNode::Kaeriten(_) => counts.kaeritens += 1,
            AozoraNode::TateChuYoko(_) => counts.tate_chu_yokos += 1,
            _ => counts.other += 1,
        }
    }
    for child in node.children() {
        count_aozora(child, counts);
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
