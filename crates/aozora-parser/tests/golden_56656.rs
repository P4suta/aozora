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
    let root = afm_parser::parse(&arena, FIXTURE, &options).root;

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
        + counts.containers
        + counts.double_rubies
        + counts.heading_promoted
        + counts.other;
    // Two shapes reduce the bracket-to-AST-node ratio below 1:1 —
    //
    // * Paired-container wrap folds each `［＃ここから…］` /
    //   `［＃ここで…終わり］` pair into one `AozoraNode::Container`,
    //   so the two source brackets reduce to a single counter bump.
    // * Heading promotion consumes the `［＃「X」は…見出し］` bracket
    //   AND detaches any companion indent marker (e.g. `［＃２字下げ］`)
    //   that decorated the heading paragraph — the heading is an
    //   independent block and does not carry paragraph-level
    //   indentation. In the 56656 fixture this moves ~48 brackets from
    //   the `indents` counter to the discarded bucket; the residual
    //   floor below reflects the real recognition depth with heading
    //   promotion in force.
    //
    // The floor covers the sum of every bracket-sourced sink so
    // adding a new recogniser cannot silently erode the total, and
    // can ratchet upward as further recognisers land.
    assert!(
        bracket_sourced >= 370,
        "bracket-sourced annotation recognition dropped to {bracket_sourced} \
         (expected >= 370); breakdown: {counts:?}"
    );

    // Independent assertion: heading promotion must actually be
    // firing on the 大/中/小 見出し brackets. Without this, a
    // regression that stopped promoting headings would still satisfy
    // the bracket-sourced floor because the reduction in
    // `heading_promoted` would be offset by a rise in `annotations`
    // (the pre-promotion catch-all). The floor is set a few counts
    // below the observed 48 to tolerate minor fixture drift.
    assert!(
        counts.heading_promoted >= 40,
        "heading promotion under 56656 dropped to {promoted} \
         (expected >= 40); breakdown: {counts:?}",
        promoted = counts.heading_promoted,
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
    containers: usize,
    double_rubies: usize,
    /// Count of `NodeValue::Heading` nodes produced by `splice_heading_hint`
    /// promoting a paragraph to a Markdown heading. A heading hint bracket
    /// consumes itself as it promotes, so it shows up under this counter
    /// rather than `annotations`.
    heading_promoted: usize,
    other: usize,
}

fn count_aozora<'a>(node: &'a AstNode<'a>, counts: &mut AozoraCounts) {
    match node.data.borrow().value {
        NodeValue::Aozora(ref boxed) => match **boxed {
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
            AozoraNode::Container(_) => counts.containers += 1,
            AozoraNode::DoubleRuby(_) => counts.double_rubies += 1,
            _ => counts.other += 1,
        },
        NodeValue::Heading(_) => counts.heading_promoted += 1,
        _ => {}
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
