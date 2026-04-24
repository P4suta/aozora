//! Property-based tests for ruby recognition, exercised end-to-end through
//! the lexer + `post_process` pipeline (`afm_parser::parse`).
//!
//! Targets the core round-trip invariants from
//! <https://www.aozora.gr.jp/annotation/ruby.html>:
//!
//! * Explicit `｜BASE《READING》` with plain reading always round-trips
//!   both literals and sets `delim_explicit = true`.
//! * When `READING` embeds a `※［＃「X」、mencode］` gaiji marker, the
//!   reading lifts to `Content::Segments` with the gaiji segment
//!   preserved and the surrounding text literals intact.
//!
//! Implicit-delimiter forms are covered by the lexer's unit tests in
//! `afm_lexer::phase3_classify`; the proptests here stay scoped to the
//! explicit form so the generated corpus of random inputs is unambiguous.

use afm_parser::{Options, parse};
use afm_syntax::{AozoraNode, Content, Segment, SegmentRef};
use afm_test_utils::generators::{hiragana_fragment, kanji_fragment};
use comrak::Arena;
use comrak::nodes::{AstNode, NodeValue};
use proptest::prelude::*;

/// Walk `root` and return the first `AozoraNode::Ruby` encountered, or
/// `None` if the tree contains no ruby. Test-only helper.
fn first_ruby<'a>(root: &'a AstNode<'a>) -> Option<afm_syntax::Ruby> {
    for n in root.descendants() {
        if let NodeValue::Aozora(ref boxed) = n.data.borrow().value
            && let AozoraNode::Ruby(ref r) = **boxed
        {
            return Some(r.clone());
        }
    }
    None
}

/// Concatenate the text segments of a [`Content`] in order. Gaiji and
/// annotation segments contribute their `description` / `raw`
/// respectively so the returned string faithfully represents what the
/// renderer writes into `<rt>…</rt>` (modulo the HTML wrappers).
///
/// Defensive against `Content::Plain` (single synthesised Text) and
/// mixed `Segments` — the iterator protocol unifies both.
fn content_to_flat_string(c: &Content) -> String {
    let mut out = String::new();
    for seg in c {
        match seg {
            SegmentRef::Text(t) => out.push_str(t),
            SegmentRef::Gaiji(g) => out.push_str(&g.description),
            SegmentRef::Annotation(a) => out.push_str(&a.raw),
            _ => {}
        }
    }
    out
}

proptest! {
    /// Explicit delimiter form with *plain* reading always round-trips
    /// base/reading exactly through the full parse pipeline and keeps
    /// the reading on the `Content::Plain` fast path (no allocation for
    /// a `Segments` run).
    #[test]
    fn explicit_ruby_round_trips(base in kanji_fragment(5), reading in hiragana_fragment(10)) {
        let input = format!("｜{base}《{reading}》");
        let arena = Arena::new();
        let opts = Options::afm_default();
        let root = parse(&arena, &input, &opts).root;
        let ruby = first_ruby(root).expect("explicit form must parse to a Ruby node");
        prop_assert_eq!(ruby.base.as_plain(), Some(base.as_str()));
        prop_assert_eq!(ruby.reading.as_plain(), Some(reading.as_str()));
        prop_assert!(ruby.delim_explicit);
    }

    /// A reading that embeds `※［＃「GAIJI」、mencode］` between two
    /// hiragana runs must surface as
    /// `Content::Segments([Text(prefix), Gaiji(..), Text(suffix)])`.
    /// Checking the three-segment shape and the gaiji description
    /// pinpoints any regression in `build_content_from_body`'s
    /// text-span bookkeeping.
    #[test]
    fn explicit_ruby_with_nested_gaiji_lifts_to_segments(
        base in kanji_fragment(4),
        prefix in hiragana_fragment(5),
        suffix in hiragana_fragment(5),
        gaiji_desc in kanji_fragment(2),
    ) {
        let input = format!(
            "｜{base}《{prefix}※［＃「{gaiji_desc}」、第3水準1-85-54］{suffix}》"
        );
        let arena = Arena::new();
        let opts = Options::afm_default();
        let root = parse(&arena, &input, &opts).root;
        let ruby = first_ruby(root).expect("ruby must parse");
        // Base stays Plain (explicit base is always a single Text event).
        prop_assert_eq!(ruby.base.as_plain(), Some(base.as_str()));
        // Reading lifts to Segments.
        let Content::Segments(ref segs) = ruby.reading else {
            prop_assert!(false, "reading must be Segments, got {:?}", ruby.reading);
            unreachable!();
        };
        prop_assert_eq!(segs.len(), 3);
        prop_assert!(matches!(&segs[0], Segment::Text(t) if &**t == prefix.as_str()));
        let Segment::Gaiji(ref g) = segs[1] else {
            prop_assert!(false, "segment 1 must be Gaiji");
            unreachable!();
        };
        prop_assert_eq!(&*g.description, gaiji_desc.as_str());
        prop_assert_eq!(g.mencode.as_deref(), Some("第3水準1-85-54"));
        prop_assert!(matches!(&segs[2], Segment::Text(t) if &**t == suffix.as_str()));
        // Flat string round-trip: concatenating the segments back yields
        // the original reading bytes (prefix + gaiji description + suffix).
        let flat = content_to_flat_string(&ruby.reading);
        prop_assert_eq!(flat, format!("{prefix}{gaiji_desc}{suffix}"));
    }
}

/// Concrete case: reading ending in `［＃ママ］` lifts to Segments
/// with a trailing Annotation segment. Pinned as a non-property test
/// because the specific `AnnotationKind::Mama` classification is
/// shape-exact (no randomisation buys coverage here).
#[test]
fn ruby_reading_with_mama_annotation_lifts_to_segments() {
    let input = "｜日本《にほん［＃ママ］》";
    let arena = Arena::new();
    let opts = Options::afm_default();
    let root = parse(&arena, input, &opts).root;
    let ruby = first_ruby(root).expect("ruby must parse");
    let Content::Segments(ref segs) = ruby.reading else {
        panic!("expected Segments, got {:?}", ruby.reading);
    };
    assert_eq!(segs.len(), 2);
    assert!(matches!(&segs[0], Segment::Text(t) if &**t == "にほん"));
    assert!(matches!(&segs[1], Segment::Annotation(_)));
}
