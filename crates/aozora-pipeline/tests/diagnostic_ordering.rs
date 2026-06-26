//! Diagnostic ordering invariants for the fused lex pipeline.
//!
//! Plan G.4 deliverable. Pins the [`Pipeline::build`] documented order:
//!
//!   sanitize → pair → classify
//!
//! Downstream tooling — IDE diagnostics, the CLI's miette renderer,
//! property tests that grep for diagnostic positions — relies on this
//! order being stable, so any change to the pipeline that re-orders
//! diagnostic emission lights up here.
//!
//! The current `Diagnostic` enum (see `crates/aozora-spec/src/diagnostic.rs`)
//! exposes no classify-stage variant for "unrecognised annotation keyword"; an
//! unknown body is folded into `DirectiveKind::Unknown` silently. We
//! therefore pin the sanitize ↔ pair ordering only, with a
//! `classify` placeholder comment marking where a future variant would
//! slot in. The insta snapshot freezes the multi-diagnostic shape
//! end-to-end.

use aozora_pipeline::lex;
use aozora_spec::{Diagnostic, DiagnosticSource, codes};
/// Ordinal position of a diagnostic in the documented pipeline order
/// (sanitize → pair → classify → "later").
///
/// Post-Phase-C the four legacy `Registry*` / `Unregistered*` /
/// `ResidualAnnotationMarker` variants are folded into
/// [`Diagnostic::Internal`] with a stable `code` payload — they
/// remain post-classify validators and still sort last.
fn phase_ordinal(d: &Diagnostic) -> u8 {
    match d.source() {
        // Source-side diagnostics — match by stable code.
        DiagnosticSource::Source => match d.code() {
            // sanitize stage.
            codes::SOURCE_CONTAINS_PUA | codes::ACCENT_DECOMPOSITION_APPLIED => 0,
            // pair stage.
            codes::UNCLOSED_BRACKET | codes::UNMATCHED_CLOSE => 2,
            // classify stage.
            codes::UNRESOLVED_GAIJI
            | codes::EMPTY_RUBY_READING
            | codes::NESTED_RUBY
            | codes::UNRECOGNISED_CONTAINER_DIRECTIVE
            | codes::TCY_TARGET_NOT_FOUND
            | codes::BOUTEN_TARGET_AMBIGUOUS
            | codes::BRACKETED_KAERITEN_NO_PAIR
            | codes::KAERITEN_OUTSIDE_KANBUN => 3,
            // Post-classify normalizer fold (same slot as the Internal
            // validators below, which also run after classify).
            codes::MISMATCHED_CONTAINER_CLOSE
            | codes::BREAK_IN_SINGLE_LINE_CONTAINER
            | codes::MISMATCHED_BOUTEN_CONTAINER => 4,
            _ => 99,
        },
        // Pipeline-internal validators run after the classify stage.
        DiagnosticSource::Internal => 4,
        // `DiagnosticSource` is `#[non_exhaustive]`. Any future
        // category lands here until classified explicitly.
        _ => 99,
    }
}

#[test]
fn phase0_then_phase2_diagnostics_are_emitted_in_pipeline_order() {
    // PUA collision (sanitize stage) + unclosed bracket (pair stage) — the
    // canonical multi-stage shape. The PUA collision is byte 0 of the
    // source so any "sort-by-position" alternative ordering would also
    // put it first; we keep the pin minimal so that a regression that
    // re-sorts diagnostics by phase ordinal is what we'd notice.
    let src = "\u{E001}［＃unclosed";
    let out = lex(src);

    let ordinals: Vec<u8> = out.diagnostics.iter().map(phase_ordinal).collect();

    // Must contain at least one sanitize-stage and one pair-stage diagnostic.
    assert!(
        ordinals.contains(&0),
        "expected a sanitize-stage diagnostic in {:?}",
        out.diagnostics
    );
    assert!(
        ordinals.contains(&2),
        "expected a pair-stage diagnostic in {:?}",
        out.diagnostics
    );

    // Ordinals must be monotonically non-decreasing (sanitize-stage entries
    // come first, then pair stage, then classify stage onward if any).
    let mut sorted = ordinals.clone();
    sorted.sort_unstable();
    assert_eq!(
        ordinals, sorted,
        "diagnostics must come back in pipeline order, got ordinals={ordinals:?} for {:?}",
        out.diagnostics
    );
}

/// Insta snapshot of the diagnostic vector for a hand-curated
/// multi-diagnostic input. Freezes the *shape* (variants, kinds, span
/// payloads) byte-for-byte so any reorder, payload drift, or
/// over/under-emission lights up as a snapshot diff.
///
/// The input combines:
///   * sanitize-stage PUA collision at position 0.
///   * pair-stage unmatched close (`］` in mid-text without an open).
///   * pair-stage unclosed bracket (`［＃...` at end of input).
#[test]
fn multi_diagnostic_snapshot_freezes_pipeline_order() {
    let src = "\u{E001}stray］then［＃tail";
    let out = lex(src);
    insta::assert_snapshot!(format!("{:#?}", out.diagnostics));
}
