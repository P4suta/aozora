//! Golden fixture — 青空文庫 card 56656 (『罪と罰』米川正夫訳).
//!
//! Runs the aozora parser pipeline against a real, densely-annotated
//! translation of Dostoevsky and asserts the M0 Spike "Tier A"
//! contract:
//!
//! 1. The parser completes without panicking on a full-length Aozora
//!    Bunko work.
//! 2. Every `［＃…］` sequence is consumed (wrapped inside an
//!    `afm-annotation` node) — no bare annotation markers leak into
//!    the rendered HTML.
//! 3. Every `｜…《…》` explicit-ruby span is recognised.

use aozora_parser::html::render_to_string;
use aozora_parser::test_support::{assert_no_bare, collect_aozora, strip_annotation_wrappers};
use aozora_syntax::AozoraNode;

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
/// (`Annotation` + `Bouten` + `PageBreak` + `SectionBreak` + …) and
/// compare against the known floors. A regression to 0 would silently
/// go undetected if we only asserted parse success.
#[test]
fn tier_a_ruby_recognition_floor() {
    let nodes = collect_aozora(FIXTURE);
    let mut counts = AozoraCounts::default();
    for node in &nodes {
        counts.add(node);
    }

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
        + counts.heading_hints
        + counts.other;
    assert!(
        bracket_sourced >= 370,
        "bracket-sourced annotation recognition dropped to {bracket_sourced} \
         (expected >= 370); breakdown: {counts:?}"
    );

    // Independent assertion: heading recognition must actually be
    // firing on the 大/中/小 見出し brackets. The floor is set a few
    // counts below the observed 48 to tolerate minor fixture drift.
    assert!(
        counts.heading_hints >= 40,
        "heading-hint recognition under 56656 dropped to {hints} \
         (expected >= 40); breakdown: {counts:?}",
        hints = counts.heading_hints,
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
    /// `［＃「X」は(大|中|小)見出し］` annotations that the lexer
    /// classified as a heading hint. After post-processing in a
    /// downstream Markdown integration these become Markdown headings;
    /// at the aozora layer they remain `HeadingHint` nodes.
    heading_hints: usize,
    other: usize,
}

impl AozoraCounts {
    fn add(&mut self, node: &AozoraNode) {
        match node {
            AozoraNode::Ruby(_) => self.rubies += 1,
            AozoraNode::Annotation(_) => self.annotations += 1,
            AozoraNode::Bouten(_) => self.boutens += 1,
            AozoraNode::PageBreak => self.page_breaks += 1,
            AozoraNode::SectionBreak(_) => self.section_breaks += 1,
            AozoraNode::Indent(_) => self.indents += 1,
            AozoraNode::AlignEnd(_) => self.align_ends += 1,
            AozoraNode::Gaiji(_) => self.gaijis += 1,
            AozoraNode::Kaeriten(_) => self.kaeritens += 1,
            AozoraNode::TateChuYoko(_) => self.tate_chu_yokos += 1,
            AozoraNode::Container(_) => self.containers += 1,
            AozoraNode::DoubleRuby(_) => self.double_rubies += 1,
            AozoraNode::HeadingHint(_) => self.heading_hints += 1,
            _ => self.other += 1,
        }
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
