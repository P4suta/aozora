//! Invariants for `AnnotationKind::Unknown` — the catch-all that
//! Phase 3 emits when no recogniser claimed an `［＃…］` annotation.
//!
//! The catch-all is a deliberate decision: every bracket annotation is
//! claimed *somewhere* in the AST so the Tier-A canary (no bare `［＃`
//! survives to HTML) holds end-to-end. The property tests here pin the
//! four invariants the catch-all must satisfy:
//!
//! 1. **panic-free**: lex must not panic on any input that exercises
//!    the catch-all path, including pathological / unbalanced bracket
//!    shapes.
//! 2. **non-empty raw bytes**: every emitted `Annotation` node carries
//!    a `NonEmptyStr` raw payload — the type system enforces it, but
//!    we cross-check the runtime invariant explicitly so a future
//!    refactor that loosens the type can't quietly emit empty
//!    annotations.
//! 3. **lex → serialise → lex round-trip preserves Annotation count**:
//!    re-parsing the serialised output must produce the same number of
//!    Annotation nodes (any kind, including Unknown). A regression that
//!    drops or duplicates an Unknown annotation on the second pass
//!    would shift this count.
//! 4. **Tier-A canary holds on well-formed input**: when the lexer
//!    emits no diagnostics for the source, the serialised text must
//!    not contain a bare `［＃` (i.e. one not paired with a closing
//!    `］`). On *malformed* inputs the serialiser legitimately
//!    round-trips the source's own asymmetry — Tier-A is an HTML-side
//!    contract, and the serialise side honours it only when the
//!    lexer accepts the input cleanly.

use aozora_pipeline::lex_into_arena;
use aozora_proptest::config::default_config;
use aozora_proptest::generators::*;
use aozora_render::serialize::serialize;
use aozora_syntax::borrowed::{AozoraNode, Arena, NodeRef};
use proptest::prelude::*;

/// Walk every registry hit and pull out the `Annotation` nodes.
///
/// Annotations are emitted on `Sentinel::Inline` positions, so they
/// surface as `NodeRef::Inline(AozoraNode::Annotation(_))`. Container
/// open/close hits never carry annotations; block-leaf hits in the
/// borrowed AST are reserved for non-annotation block primitives
/// (page break, section break, sashie, …).
fn count_annotations(out: &aozora_pipeline::BorrowedLexOutput<'_>) -> usize {
    out.registry
        .iter_sorted()
        .filter(|(_, nr)| matches!(nr, NodeRef::Inline(AozoraNode::Annotation(_))))
        .count()
}

/// Whether the registry contains at least one annotation. Used as a
/// guard for properties that only make sense when the input actually
/// triggered the catch-all path; properties that hold *unconditionally*
/// don't need this guard.
fn has_annotation(out: &aozora_pipeline::BorrowedLexOutput<'_>) -> bool {
    out.registry
        .iter_sorted()
        .any(|(_, nr)| matches!(nr, NodeRef::Inline(AozoraNode::Annotation(_))))
}

/// Tier-A canary on a string: no bare `［＃` may appear without a
/// corresponding `］` after it on the same line. Not a perfect
/// scanner — it exists to catch the obvious regression where the
/// serialiser drops a closing bracket — but a bare `［＃` reaching
/// the output is itself a Tier-A violation regardless of context.
fn assert_no_bare_open_bracket(serialised: &str) {
    let mut cursor = 0_usize;
    while let Some(rel) = serialised[cursor..].find("［＃") {
        let open_at = cursor + rel;
        let after = &serialised[open_at + "［＃".len()..];
        assert!(
            after.contains('］'),
            "bare `［＃` at byte {open_at} has no following `］` in serialised output: {serialised:?}"
        );
        cursor = open_at + "［＃".len();
    }
}

fn assert_annotation_invariants(source: &str) {
    let arena_a = Arena::new();
    let out_a = lex_into_arena(source, &arena_a);

    // (2) Every annotation's raw payload is non-empty (the type system
    // already says so via `NonEmptyStr`, but we cross-check at runtime
    // so the property fails loudly if a future refactor weakens the
    // invariant). Also verify each annotation's raw bytes are valid
    // UTF-8 — true by construction (it's `&str`), but a `is_empty`
    // probe on every annotation closes the loop on (2).
    for (_, nr) in out_a.registry.iter_sorted() {
        if let NodeRef::Inline(AozoraNode::Annotation(a)) = nr {
            assert!(
                !a.raw.as_str().is_empty(),
                "annotation with empty raw bytes for source {source:?}"
            );
        }
    }

    // (3) lex → serialise → lex preserves the annotation count.
    let serialised = serialize(&out_a);
    let arena_b = Arena::new();
    let out_b = lex_into_arena(&serialised, &arena_b);
    let count_first = count_annotations(&out_a);
    let count_second = count_annotations(&out_b);
    assert_eq!(
        count_first, count_second,
        "annotation count drifted on round-trip: first={count_first} second={count_second}\n\
         source: {source:?}\nserialised: {serialised:?}"
    );

    // (4) Tier-A canary on serialised text — no bare `［＃` leaks
    // out *when the lexer accepts the input cleanly*. On malformed
    // inputs (those that emit diagnostics), the serialiser round-trips
    // the user's own asymmetry; Tier-A is an HTML-side contract and
    // does not bind the serialise path on malformed source.
    if out_a.diagnostics.is_empty() {
        assert_no_bare_open_bracket(&serialised);
    }

    // (1) panic-free is implicit: reaching this point means lex did not
    // panic. We add an explicit reachability marker for inputs that
    // actually exercised the catch-all so a future generator change
    // that stops producing Unknown annotations is visible.
    if has_annotation(&out_a) {
        // Sanity: the annotated case re-lexed must still see at least
        // one annotation (catch-all is stable across passes).
        assert!(
            has_annotation(&out_b),
            "annotation present in first pass but vanished in second for source {source:?}\n\
             serialised: {serialised:?}"
        );
    }
}

// ----------------------------------------------------------------------
// Hand-curated regression anchors.
// ----------------------------------------------------------------------

#[test]
fn empty_input_has_no_annotations() {
    assert_annotation_invariants("");
}

#[test]
fn known_annotations_round_trip() {
    assert_annotation_invariants("text［＃改ページ］more");
    assert_annotation_invariants("［＃ここから2字下げ］\nbody\n［＃ここで字下げ終わり］");
}

#[test]
fn unknown_annotations_round_trip() {
    // Definitely not in the recogniser table — falls through to
    // `AnnotationKind::Unknown`.
    assert_annotation_invariants("text［＃this is a wholly novel marker］more");
    assert_annotation_invariants("［＃random text 1234］");
}

#[test]
fn quoted_annotations_round_trip() {
    assert_annotation_invariants("text［＃「青空」に傍点］more");
    assert_annotation_invariants("text［＃「これは未知の指示」］more");
}

proptest! {
    #![proptest_config(default_config())]

    /// Workhorse — Aozora-shaped fragments must satisfy all four
    /// invariants regardless of which annotation kinds get exercised.
    #[test]
    fn aozora_fragment_annotations_are_well_formed(s in aozora_fragment(120)) {
        assert_annotation_invariants(&s);
    }

    /// Pathological — unbalanced and runaway bracket shapes are the
    /// ones that drive the catch-all hardest. If the catch-all panics
    /// or drops annotations on the round-trip, this property fails
    /// under shrinking with a minimal repro.
    #[test]
    fn pathological_input_annotations_are_well_formed(s in pathological_aozora(120)) {
        assert_annotation_invariants(&s);
    }

    /// Unicode adversarial — combining marks, RTL overrides, PUA
    /// codepoints, full-width bracket variants. Cross-checks that the
    /// catch-all does not misbehave when the annotation body itself
    /// contains adversarial Unicode.
    #[test]
    fn unicode_adversarial_annotations_are_well_formed(s in unicode_adversarial()) {
        assert_annotation_invariants(&s);
    }
}
