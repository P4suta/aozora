//! Public-API totality + idempotence for [`Document::snapshot`].
//!
//! Two complementary properties on the top-facade entry point:
//!
//! 1. **Totality**: [`Document::snapshot`] must not panic on any input.
//!    The lex pipeline emits diagnostics rather than failing, and the
//!    facade is the sole entry point for FFI / WASM / Python drivers
//!    that hand user input straight to the parser. A panic here is a
//!    denial-of-service surface for every downstream binding.
//!
//! 2. **Parse → serialise → parse converges**: the second
//!    `Document::snapshot` over the serialised output of the first
//!    yields a tree whose own `serialize()` matches. Equivalent to
//!    "the renderer's `serialize` is a parser fixed point" — a
//!    quieter way to surface bugs that mutate documents on round-trip
//!    (whitespace drift, character substitutions). Functionally
//!    overlaps the `aozora-render` `property_serialize_idempotent`
//!    test, but exercised here through the **public API only** to
//!    catch facade regressions (e.g. a `Document` that builds an
//!    inconsistent arena).

use aozora_proptest::config::default_config;
use aozora_proptest::generators::*;
use proptest::prelude::*;
use std::{iter, mem};

fn parse_serialise_parse(source: &str) -> (String, String) {
    let doc = aozora::parse(source.to_owned()).expect("source fits parser span limit");
    let tree = doc.snapshot();
    let first = tree.to_source();
    let doc2 = aozora::parse(first.clone()).expect("source fits parser span limit");
    let tree2 = doc2.snapshot();
    let second = tree2.to_source();
    (first, second)
}

fn assert_facade_round_trip_converges(source: &str) {
    let (first, second) = parse_serialise_parse(source);
    assert_eq!(
        first, second,
        "facade round-trip diverges on second pass for source {source:?}\n\
         after 1st: {first:?}\n\
         after 2nd: {second:?}"
    );
}

fn boundary_at(source: &str, numerator: usize, denominator: usize) -> usize {
    let boundaries = source
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(iter::once(source.len()))
        .collect::<Vec<_>>();
    boundaries[boundaries.len().saturating_sub(1) * numerator / denominator]
}

fn selected_boundary(source: &str, selector: u16) -> usize {
    let boundaries = source
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(iter::once(source.len()))
        .collect::<Vec<_>>();
    boundaries[usize::from(selector) % boundaries.len()]
}

fn assert_snapshots_equivalent(actual: &aozora::Snapshot, expected: &aozora::Snapshot) {
    assert_eq!(
        actual.source(),
        expected.source(),
        "source projection differs"
    );
    assert_eq!(
        actual.normalized_source(),
        expected.normalized_source(),
        "normalized source projection differs"
    );
    assert_eq!(
        actual
            .diagnostics()
            .iter()
            .map(|diagnostic| (
                diagnostic.code(),
                diagnostic.severity(),
                diagnostic.source(),
                diagnostic.span(),
                diagnostic.to_string(),
            ))
            .collect::<Vec<_>>(),
        expected
            .diagnostics()
            .iter()
            .map(|diagnostic| (
                diagnostic.code(),
                diagnostic.severity(),
                diagnostic.source(),
                diagnostic.span(),
                diagnostic.to_string(),
            ))
            .collect::<Vec<_>>(),
        "diagnostics projection differs"
    );
    assert_eq!(actual.nodes(), expected.nodes(), "nodes projection differs");
    assert_eq!(actual.pairs(), expected.pairs(), "pairs projection differs");
    assert_eq!(
        actual.container_pairs(),
        expected.container_pairs(),
        "container pairs projection differs"
    );
    assert_eq!(
        actual.literal_markup(),
        expected.literal_markup(),
        "literal markup projection differs"
    );
    assert_eq!(
        actual.directives().collect::<Vec<_>>(),
        expected.directives().collect::<Vec<_>>(),
        "directives projection differs"
    );
    assert_eq!(
        actual.rubies().collect::<Vec<_>>(),
        expected.rubies().collect::<Vec<_>>(),
        "rubies projection differs"
    );
    assert_eq!(
        actual.gaiji_resolutions(),
        expected.gaiji_resolutions(),
        "gaiji projection differs"
    );
    assert_eq!(
        actual.to_html(),
        expected.to_html(),
        "HTML projection differs"
    );
    assert_eq!(
        actual.to_source(),
        expected.to_source(),
        "source render projection differs"
    );
    assert_eq!(
        actual.to_source_verbatim(),
        expected.to_source_verbatim(),
        "verbatim source projection differs"
    );
}

// ----------------------------------------------------------------------
// Hand-curated regression anchors.
// ----------------------------------------------------------------------

#[test]
fn empty_input_round_trips() {
    assert_facade_round_trip_converges("");
}

#[test]
fn plain_text_round_trips() {
    assert_facade_round_trip_converges("Hello, world.");
    assert_facade_round_trip_converges("こんにちは。\n\n本日は晴れ。");
}

#[test]
fn ruby_round_trips() {
    assert_facade_round_trip_converges("｜青梅《おうめ》");
    assert_facade_round_trip_converges("青梅《おうめ》");
}

#[test]
fn paired_container_round_trips() {
    assert_facade_round_trip_converges("［＃ここから2字下げ］\nbody\n［＃ここで字下げ終わり］");
}

#[test]
fn incremental_rule_paragraph_edits_match_a_fresh_parse() {
    let mut source = "｜｜、※1234\n------------\n\nかんじ1234、\n\nかんじ\n------------\n\n\
        ----------------------------------------------------------------------｜1234\n\
        かんじ改丁、改丁------------改丁｜------------｜［＃［＃改丁かんじ、［＃\n\n\
        ------------\n\n\
        、-----------------------------------かんじかんじ-----------------------------------※\n\
        ※------------"
        .to_owned();
    let operations = [
        (14_127, 24_374, "改ページ［＃"),
        (
            56_327,
            9_110,
            "------------------------］［＃］\n\nかんじ、------------、改ページ",
        ),
        (7_647, 56_338, "\n｜"),
        (21_725, 612, "｜"),
    ];
    let mut incremental = aozora::parse(source.clone()).expect("source fits parser span limit");
    for (step, (left, right, replacement)) in operations.into_iter().enumerate() {
        let prior = source.clone();
        let mut start = selected_boundary(&source, left);
        let mut end = selected_boundary(&source, right);
        if start > end {
            mem::swap(&mut start, &mut end);
        }
        source.replace_range(start..end, replacement);
        incremental
            .edit([aozora::TextEdit::new(start..end, replacement)])
            .expect("regression edit is valid");
        let expected = aozora::parse(source.clone())
            .expect("source fits parser span limit")
            .snapshot();
        let actual = incremental.snapshot();
        assert_eq!(
            actual.to_html(),
            expected.to_html(),
            "step {step}, range {start}..{end}, prior {prior:?}, source {source:?}"
        );
        assert_snapshots_equivalent(&actual, &expected);
    }
}

proptest! {
    #![proptest_config(default_config())]

    /// `Document::snapshot` is total over the workhorse Aozora fragment
    /// distribution. A panic here is a DoS surface for every public
    /// caller — proptest is the decisive way to catch one.
    #[test]
    fn aozora_fragment_parse_is_total(s in aozora_fragment(120)) {
        let doc = aozora::parse(s).expect("source fits parser span limit");
        let _tree = doc.snapshot();
    }

    /// Parse → serialise → parse converges on the second pass. The
    /// serialiser must produce a parser fixed point.
    #[test]
    fn aozora_fragment_round_trip_converges(s in aozora_fragment(120)) {
        assert_facade_round_trip_converges(&s);
    }

    /// Pathological / unbalanced inputs — public parse must stay
    /// total even when diagnostics fire.
    #[test]
    fn pathological_input_parse_is_total(s in pathological_aozora(120)) {
        let doc = aozora::parse(s).expect("source fits parser span limit");
        let _tree = doc.snapshot();
    }

    /// Unicode adversarial — combining marks, RTL overrides, PUA
    /// bytes that the lexer reserves for sentinel use. Public parse
    /// must stay total and emit `SourceContainsPua` diagnostics
    /// rather than panicking.
    #[test]
    fn unicode_adversarial_parse_is_total(s in unicode_adversarial()) {
        let doc = aozora::parse(s).expect("source fits parser span limit");
        let _tree = doc.snapshot();
    }

    #[test]
    fn batched_edits_match_a_fresh_parse(
        source in aozora_fragment(120),
        first_replacement in aozora_fragment(12),
        second_replacement in aozora_fragment(12),
    ) {
        let first = boundary_at(&source, 1, 3);
        let second = boundary_at(&source, 2, 3);
        let expected_source = format!(
            "{}{}{}{}{}",
            &source[..first],
            first_replacement,
            &source[first..second],
            second_replacement,
            &source[second..],
        );
        let mut incremental = aozora::parse(source).expect("source fits parser span limit");
        incremental
            .edit([
                aozora::TextEdit::new(first..first, first_replacement),
                aozora::TextEdit::new(second..second, second_replacement),
            ])
            .expect("generated edits are sorted character-boundary insertions");
        let actual = incremental.snapshot();
        let expected = aozora::parse(expected_source)
            .expect("source fits parser span limit")
            .snapshot();
        assert_snapshots_equivalent(&actual, &expected);
    }

    #[test]
    fn replacement_batches_match_a_fresh_parse(
        source in aozora_fragment(120),
        selectors in any::<[u16; 4]>(),
        first_replacement in aozora_fragment(12),
        second_replacement in aozora_fragment(12),
    ) {
        let mut boundaries = selectors.map(|selector| selected_boundary(&source, selector));
        boundaries.sort_unstable();
        let [first_start, first_end, second_start, second_end] = boundaries;
        let mut expected_source = source.clone();
        expected_source.replace_range(second_start..second_end, &second_replacement);
        expected_source.replace_range(first_start..first_end, &first_replacement);
        let mut incremental = aozora::parse(source).expect("source fits parser span limit");
        incremental
            .edit([
                aozora::TextEdit::new(first_start..first_end, first_replacement),
                aozora::TextEdit::new(second_start..second_end, second_replacement),
            ])
            .expect("sorted character-boundary replacements");
        let actual = incremental.snapshot();
        let expected = aozora::parse(expected_source)
            .expect("source fits parser span limit")
            .snapshot();
        assert_snapshots_equivalent(&actual, &expected);
    }

    #[test]
    fn arbitrary_edit_sequences_match_a_fresh_parse(
        source in aozora_fragment(120),
        operations in prop::collection::vec(
            (any::<u16>(), any::<u16>(), aozora_fragment(12)),
            1..8,
        ),
    ) {
        let mut expected_source = source.clone();
        let mut incremental = aozora::parse(source).expect("source fits parser span limit");
        for (left, right, replacement) in operations {
            let mut start = selected_boundary(&expected_source, left);
            let mut end = selected_boundary(&expected_source, right);
            if start > end {
                mem::swap(&mut start, &mut end);
            }
            expected_source.replace_range(start..end, &replacement);
            incremental
                .edit([aozora::TextEdit::new(start..end, replacement)])
                .expect("generated character-boundary replacement");
            let actual = incremental.snapshot();
            let expected = aozora::parse(expected_source.as_str())
                .expect("source fits parser span limit")
                .snapshot();
            assert_snapshots_equivalent(&actual, &expected);
        }
    }
}
