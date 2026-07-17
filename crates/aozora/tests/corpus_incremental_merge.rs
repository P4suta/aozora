//! Load-bearing gate for incremental re-parse (#237).
//!
//! Proves, over every document in `AOZORA_CORPUS_ROOT`, that the
//! **diagnostics-only** incremental engine
//! [`aozora::reparse_incremental_diagnostics_only`] — the LSP's per-keystroke
//! hot path — splices diagnostics **byte-for-byte equal** to a from-scratch
//! parse of the edited text. A single deterministic plain-character insertion
//! near each document's midpoint exercises the splice fast path on global-free
//! documents and the full-parse fallback elsewhere; the fast-path count is
//! asserted non-zero so the gate actually drives the splice. Any edit the splice
//! cannot prove byte-identical returns `None` and falls back to a full parse
//! (trivially correct).
//!
//! Skipped silently when `AOZORA_CORPUS_ROOT` is unset; never hard-fails on
//! a missing corpus (mirrors `corpus_sweep`).
//!
//! The incremental engine is `unstable-internals`-gated (`#[doc(hidden)]`, no
//! semver contract); this whole differential gate compiles only with that
//! feature on.
#![cfg(feature = "unstable-internals")]

use aozora::encoding::decode_auto;
use aozora::{DiagBaseRef, Diagnostic, Document, PieceSeq, reparse_incremental_diagnostics_only};

/// #237 Tier 1: the **diagnostics-only** incremental engine
/// ([`aozora::reparse_incremental_diagnostics_only`]) — the LSP's per-keystroke
/// hot path — must produce diagnostics byte-identical to a from-scratch parse of
/// the edited text, for every corpus document whose midpoint insertion is a
/// sanitize fixed point.
///
/// The harness takes each document's sanitized fixed-point buffer as the cached
/// baseline and applies a single midpoint plain insertion. Divergences are
/// collected and reported together rather than failing fast, and the fast-path
/// count is asserted non-zero so the gate actually drives the engine.
#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one corpus differential harness: per-doc baseline + diagnostics-only splice, all over the same loop"
)]
fn reparse_diagnostics_only_equals_full_parse() {
    let Some(source) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set; skipping diagnostics-only incremental gate");
        return;
    };

    let mut count: usize = 0;
    let mut fast_path: usize = 0;
    let mut fallback: usize = 0;
    let mut diverged: Vec<String> = Vec::new();

    for item in source.iter() {
        let item = item.expect("corpus iteration must not error");
        let Ok(text) = decode_auto(&item.bytes) else {
            continue;
        };
        let text = text.as_ref();
        if text.is_empty() {
            continue;
        }

        // The incremental engine operates entirely in **sanitized** space (the
        // raw↔sanitized bridge is a later wiring PR): `cached` must therefore be
        // the parse of already-sanitized text, not the raw corpus bytes. Parsing
        // the raw text would leave sanitize-stage diagnostics
        // (`AccentDecompositionApplied` / `SourceContainsPua`) in `cached` that a
        // re-parse of the sanitized buffer never reproduces — a harness
        // asymmetry, not a splice property. So sanitize once, then take the parse
        // of the sanitized buffer as the cached baseline.
        let san = Document::new(text).lex().sanitized;
        if san.is_empty() {
            continue;
        }
        let cached = Document::new(san.as_str()).lex();
        // Skip documents whose sanitized buffer is not itself a sanitize fixed
        // point (sanitize is non-idempotent for them): the incremental contract
        // assumes a stable sanitized baseline, and production would full-parse.
        if cached.sanitized != san {
            continue;
        }

        // A plain ASCII insertion at a char boundary near the sanitized midpoint.
        let mut mid = san.len() / 2;
        while mid < san.len() && !san.is_char_boundary(mid) {
            mid += 1;
        }
        let new_san = format!("{}x{}", &san[..mid], &san[mid..]);

        // Idempotence precheck: only edits that are sanitize fixed points are
        // representative — production would full-parse a non-fixed-point edit.
        let full = Document::new(new_san.as_str()).lex();
        if full.sanitized != new_san {
            continue;
        }

        let pieces = PieceSeq::from_contiguous(
            &cached.source_nodes,
            &cached.pairs,
            &cached.diagnostics,
            cached.sanitized_len,
        );
        let Some(diag) = reparse_incremental_diagnostics_only(
            DiagBaseRef::from_cached(&cached, &pieces),
            &new_san,
            mid..mid,
        ) else {
            fallback += 1;
            count += 1;
            continue;
        };
        fast_path += 1;
        count += 1;
        // The diagnostics-only splice returns the maintained `PieceSeq`; flatten
        // it (the bridge by which this gate keeps comparing resolved surfaces).
        let diag_diags = diag.pieces.collect_diagnostics();

        // Diagnostics byte-identical to a full parse (the production contract).
        if sorted_debug(diag_diags) != sorted_debug(full.diagnostics.clone()) {
            diverged.push(format!(
                "{}: diagnostics-only multiset != full parse",
                item.label
            ));
        }
    }

    eprintln!(
        "diagnostics-only incremental gate: {count} docs edited, {fast_path} fast-path, \
         {fallback} fallback, {} diverged",
        diverged.len(),
    );
    assert!(
        diverged.is_empty(),
        "{} document(s) where the diagnostics-only splice diverged:\n  {}",
        diverged.len(),
        diverged.join("\n  "),
    );
    assert!(
        fast_path > 0,
        "the diagnostics-only gate must exercise the fast path at least once \
         (got {fast_path}); a perpetual fallback proves nothing",
    );
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
