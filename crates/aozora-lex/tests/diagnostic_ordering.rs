//! Diagnostic ordering invariants for the fused lex pipeline.
//!
//! Plan G.4 deliverable. Pins the [`Pipeline::build`] documented order:
//!
//!   Phase 0 (sanitize) → Phase 2 (pair) → Phase 3 (classify)
//!
//! Downstream tooling — IDE diagnostics, the CLI's miette renderer,
//! property tests that grep for diagnostic positions — relies on this
//! order being stable, so any change to the pipeline that re-orders
//! diagnostic emission lights up here.
//!
//! The current `Diagnostic` enum (see `crates/aozora-spec/src/diagnostic.rs`)
//! exposes no Phase-3 variant for "unrecognised annotation keyword"; an
//! unknown body is folded into `AnnotationKind::Unknown` silently. We
//! therefore pin the Phase 0 ↔ Phase 2 ordering only, with a
//! `Phase 3` placeholder comment marking where a future variant would
//! slot in. The insta snapshot freezes the multi-diagnostic shape
//! end-to-end.

use aozora_lex::lex_into_arena;
use aozora_spec::{Diagnostic, DiagnosticSource, codes};
use aozora_syntax::borrowed::Arena;

/// Ordinal position of a diagnostic in the documented pipeline order
/// (Phase 0 → Phase 2 → Phase 3 → "later").
///
/// Post-Phase-C the four legacy `Registry*` / `Unregistered*` /
/// `ResidualAnnotationMarker` variants are folded into
/// [`Diagnostic::Internal`] with a stable `code` payload — they
/// remain post-Phase-3 validators and still sort last.
fn phase_ordinal(d: &Diagnostic) -> u8 {
    match d.source() {
        // Source-side diagnostics — match by stable code.
        DiagnosticSource::Source => match d.code() {
            // Phase 0: sanitize.
            codes::SOURCE_CONTAINS_PUA => 0,
            // Phase 2: pair.
            codes::UNCLOSED_BRACKET | codes::UNMATCHED_CLOSE => 2,
            _ => 99,
        },
        // Pipeline-internal validators run after Phase 3.
        DiagnosticSource::Internal => 4,
        // `DiagnosticSource` is `#[non_exhaustive]`. Any future
        // category lands here until classified explicitly.
        _ => 99,
    }
}

#[test]
fn phase0_then_phase2_diagnostics_are_emitted_in_pipeline_order() {
    // PUA collision (Phase 0) + unclosed bracket (Phase 2) — the
    // canonical multi-phase shape. The PUA collision is byte 0 of the
    // source so any "sort-by-position" alternative ordering would also
    // put it first; we keep the pin minimal so that a regression that
    // re-sorts diagnostics by phase ordinal is what we'd notice.
    let src = "\u{E001}［＃unclosed";
    let arena = Arena::new();
    let out = lex_into_arena(src, &arena);

    let ordinals: Vec<u8> = out.diagnostics.iter().map(phase_ordinal).collect();

    // Must contain at least one Phase 0 and one Phase 2 diagnostic.
    assert!(
        ordinals.contains(&0),
        "expected a Phase 0 diagnostic in {:?}",
        out.diagnostics
    );
    assert!(
        ordinals.contains(&2),
        "expected a Phase 2 diagnostic in {:?}",
        out.diagnostics
    );

    // Ordinals must be monotonically non-decreasing (Phase 0 entries
    // come first, then Phase 2, then Phase 3+ if any).
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
///   * Phase 0 PUA collision at position 0.
///   * Phase 2 unmatched close (`］` in mid-text without an open).
///   * Phase 2 unclosed bracket (`［＃...` at end of input).
#[test]
fn multi_diagnostic_snapshot_freezes_pipeline_order() {
    let src = "\u{E001}stray］then［＃tail";
    let arena = Arena::new();
    let out = lex_into_arena(src, &arena);
    insta::assert_snapshot!(format!("{:#?}", out.diagnostics));
}
