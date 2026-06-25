//! Load-bearing gate for incremental re-parse (#237, Stage A).
//!
//! Proves two things over every document in `AOZORA_CORPUS_ROOT`:
//!
//! 1. **Reassembly equivalence.** [`aozora::SegmentedParse`]'s per-segment
//!    locals (rebased) plus its whole-document-scoped diagnostics equal a
//!    whole-document parse as a positional multiset. Because the merge is
//!    `max(local, whole-scoped)` per diagnostic, an exact match means the
//!    segments never invent or misplace a diagnostic (no phantoms).
//! 2. **Bounded non-locality.** The only diagnostics a segment cannot
//!    reproduce locally — and so must be carried from the whole-document
//!    parse — are the documented whole-document-scoped class
//!    (forward-reference bouten ambiguity). A new non-local diagnostic
//!    surfacing here fails the gate so it gets a deliberate review.
//!
//! Skipped silently when `AOZORA_CORPUS_ROOT` is unset; never hard-fails on
//! a missing corpus (mirrors `corpus_sweep`).

use aozora::{Diagnostic, Document, SegmentedParse};
use aozora_encoding::decode_auto;

/// Diagnostic variants whose computation depends on the whole document
/// (forward-reference resolution + end-of-document kaeriten pairing) and so
/// cannot be reproduced by an isolated segment. Keep in sync with
/// `aozora::segmented::is_whole_document_scoped`.
const WHOLE_DOCUMENT_SCOPED: &[&str] = &[
    "BoutenTargetAmbiguous",
    "TcyTargetNotFound",
    "UnresolvedGaiji",
    "UnrecognisedContainerDirective",
    "BracketedKaeritenNoPair",
    "KaeritenOutsideKanbun",
    "MismatchedContainerClose",
    "MismatchedBoutenContainer",
];

#[test]
fn segmented_merge_equals_whole_doc_parse() {
    let Some(source) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set; skipping incremental-merge gate");
        return;
    };

    let mut count: usize = 0;
    let mut segmented: usize = 0;
    let mut with_scoped: usize = 0;
    // Collect every problem rather than failing on the first.
    let mut diverged: Vec<String> = Vec::new();
    let mut unexpected_scoped: Vec<String> = Vec::new();

    for item in source.iter() {
        let item = item.expect("corpus iteration must not error");

        let Ok(text) = decode_auto(&item.bytes) else {
            eprintln!("skip (neither UTF-8 nor Shift_JIS): {}", item.label);
            continue;
        };

        let whole = Document::new(text.as_ref());
        let whole_diags = sorted_debug(whole.parse().diagnostics().to_vec());

        let seg = SegmentedParse::of(text.as_ref());
        if seg.is_segmented() {
            segmented += 1;
        }
        if !seg.whole_document_scoped().is_empty() {
            with_scoped += 1;
        }

        // (1) reassembly equivalence
        let merged_diags = sorted_debug(seg.merged_diagnostics());
        if whole_diags != merged_diags {
            diverged.push(format!(
                "{} (segments={}): whole={:?} merged={:?}",
                item.label,
                seg.segment_count(),
                whole_diags,
                merged_diags,
            ));
        }

        // (2) every carried diagnostic is of a documented whole-doc-scoped
        // variant
        for d in seg.whole_document_scoped() {
            let variant = variant_name(d);
            if !WHOLE_DOCUMENT_SCOPED.contains(&variant) {
                unexpected_scoped.push(format!("{}: {variant}", item.label));
            }
        }

        count += 1;
    }

    eprintln!(
        "incremental-merge gate: {count} docs walked, {segmented} multi-segment, \
         {with_scoped} with whole-document-scoped diagnostics"
    );

    let mut problems = Vec::new();
    if !diverged.is_empty() {
        problems.push(format!(
            "{} document(s) where reassembled merge != whole-doc parse:\n  {}",
            diverged.len(),
            diverged.join("\n  "),
        ));
    }
    if !unexpected_scoped.is_empty() {
        problems.push(format!(
            "{} undocumented whole-document-scoped diagnostic(s) — add to WHOLE_DOCUMENT_SCOPED \
             and the `aozora::segmented` module docs after review:\n  {}",
            unexpected_scoped.len(),
            unexpected_scoped.join("\n  "),
        ));
    }
    assert!(problems.is_empty(), "\n{}", problems.join("\n\n"));
}

/// Diagnostics sorted by position then debug string — a canonical positional
/// multiset ordering for comparison.
fn sorted_debug(mut diags: Vec<Diagnostic>) -> Vec<String> {
    diags.sort_by(|a, b| {
        let (sa, sb) = (a.span(), b.span());
        (sa.start, sa.end)
            .cmp(&(sb.start, sb.end))
            .then_with(|| format!("{a:?}").cmp(&format!("{b:?}")))
    });
    diags.iter().map(|d| format!("{d:?}")).collect()
}

/// The variant name of a diagnostic (the leading identifier of its debug
/// representation), e.g. `"BoutenTargetAmbiguous"`.
fn variant_name(d: &Diagnostic) -> &'static str {
    // Match the variants directly so the name is a compile-time constant.
    match d {
        Diagnostic::SourceContainsPua { .. } => "SourceContainsPua",
        Diagnostic::UnclosedBracket { .. } => "UnclosedBracket",
        Diagnostic::UnmatchedClose { .. } => "UnmatchedClose",
        Diagnostic::AccentDecompositionApplied { .. } => "AccentDecompositionApplied",
        Diagnostic::UnresolvedGaiji { .. } => "UnresolvedGaiji",
        Diagnostic::MismatchedContainerClose { .. } => "MismatchedContainerClose",
        Diagnostic::EmptyRubyReading { .. } => "EmptyRubyReading",
        Diagnostic::NestedRuby { .. } => "NestedRuby",
        Diagnostic::UnrecognisedContainerDirective { .. } => "UnrecognisedContainerDirective",
        Diagnostic::TcyTargetNotFound { .. } => "TcyTargetNotFound",
        Diagnostic::BoutenTargetAmbiguous { .. } => "BoutenTargetAmbiguous",
        Diagnostic::BreakInSingleLineContainer { .. } => "BreakInSingleLineContainer",
        Diagnostic::BracketedKaeritenNoPair { .. } => "BracketedKaeritenNoPair",
        Diagnostic::KaeritenOutsideKanbun { .. } => "KaeritenOutsideKanbun",
        Diagnostic::MismatchedBoutenContainer { .. } => "MismatchedBoutenContainer",
        Diagnostic::Internal { .. } => "Internal",
        _ => "Unknown",
    }
}
