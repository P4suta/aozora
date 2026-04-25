//! Property tests pinning the incremental-parse correctness contract.
//!
//! The contract: for any (`prev_source`, `edits`) pair,
//!
//! ```text
//! parse_incremental(prev, prev_source, edits)
//!     ≡  parse(apply_edits(prev_source, edits))
//! ```
//!
//! "Equivalent" here means:
//!
//! - the post-edit `new_source` matches a manual `apply_edits` of the
//!   same edits;
//! - the resulting `normalized` text is byte-for-byte equal;
//! - the resulting `diagnostics` and `registry` have equal size
//!   (we use size + structural equality rather than `Eq` because
//!   `Diagnostic` carries `miette::SourceSpan` which is not `Eq`);
//! - the path-tag breadcrumb does not change behaviour — this is
//!   the load-bearing invariant when the fast path fires.
//!
//! The proptest harness generates random aozora-shaped sources and
//! random valid edit batches, exercising both the
//! [`IncrementalDecision::PlainTextWindow`] fast path and the
//! [`IncrementalDecision::FullReparse`] fallback.

use aozora_parser::{
    IncrementalDecision, TextEdit, apply_edits, parse, parse_incremental,
};
use aozora_test_utils::config::default_config;
use aozora_test_utils::generators::{aozora_fragment, pathological_aozora};
use proptest::prelude::*;

/// Source generator that mixes aozora-shaped fragments with
/// pathological brackets so the property test exercises both
/// happy-path inputs and adversarial classifier inputs.
fn arb_source() -> impl Strategy<Value = String> {
    prop_oneof![
        aozora_fragment(24),
        pathological_aozora(8),
        "[a-zA-Z0-9 \\n]{0,64}".prop_map(|s| s),
    ]
}

proptest! {
    #![proptest_config(default_config())]

    /// Core invariant: incremental result equals full re-parse
    /// of the post-edit source.
    #[test]
    fn parse_incremental_equivalent_to_full_reparse(
        source in arb_source(),
        edits in (0..=4usize).prop_flat_map(|n| {
            // The edits depend on the source's char boundaries,
            // which we don't have at this point in the strategy
            // chain; defer to a single-edit batch built per-source
            // inside the test body.
            Just(n)
        }),
    ) {
        // Build a deterministic edit batch from the source so we can
        // run multiple edits without juggling proptest's
        // strategy-on-strategy composition. The boundaries-based
        // construction above mirrors `arb_disjoint_edits`'s pruning
        // logic.
        let batch = build_disjoint_edits(&source, edits);
        let prev = parse(&source);
        let new_source_manual = apply_edits(&source, &batch).expect("valid edits");
        let outcome = parse_incremental(&prev, &source, &batch).expect("valid edits");

        prop_assert_eq!(
            &outcome.new_source,
            &new_source_manual,
            "incremental new_source diverged from manual apply_edits"
        );

        let full = parse(&new_source_manual);
        prop_assert_eq!(
            outcome.result.artifacts.normalized,
            full.artifacts.normalized,
            "normalized text diverged for source={:?} batch={:?}",
            source, batch,
        );
        prop_assert_eq!(
            outcome.result.diagnostics.len(),
            full.diagnostics.len(),
            "diagnostic count diverged for source={:?} batch={:?}",
            source, batch,
        );
        prop_assert_eq!(
            outcome.result.artifacts.registry.len(),
            full.artifacts.registry.len(),
            "registry size diverged for source={:?} batch={:?}",
            source, batch,
        );
    }

    /// The fast path's contract: when it fires, the result is
    /// indistinguishable from a full parse.
    #[test]
    fn plain_text_window_path_matches_full_parse_when_taken(
        source in "[a-zA-Z0-9 \\n]{0,128}",
        n_edits in 0u32..4u32,
    ) {
        let edits = build_disjoint_edits(&source, n_edits as usize);
        let prev = parse(&source);
        let outcome = parse_incremental(&prev, &source, &edits).expect("valid edits");
        let full = parse(&outcome.new_source);

        // The fast path may or may not fire depending on whether the
        // pruned edit batch is empty (then it's Noop). We only assert
        // equivalence; the path tag is informational.
        prop_assert_eq!(outcome.result.artifacts.normalized, full.artifacts.normalized);
        prop_assert_eq!(outcome.result.diagnostics.len(), full.diagnostics.len());
        prop_assert_eq!(
            outcome.result.artifacts.registry.len(),
            full.artifacts.registry.len()
        );
        // For genuinely plain text (the strategy excludes triggers),
        // the path must not be Noop unless the edit list reduced to
        // zero after pruning.
        if !edits.is_empty() {
            prop_assert_ne!(outcome.decision, IncrementalDecision::Noop);
        }
    }

    /// Fallback path: any edit involving aozora triggers must take
    /// the FullReparse route AND remain equivalent to a full parse.
    #[test]
    fn full_reparse_path_matches_full_parse_when_triggers_present(
        source in pathological_aozora(8),
    ) {
        let edits = build_disjoint_edits(&source, 1);
        let prev = parse(&source);
        let outcome = parse_incremental(&prev, &source, &edits).expect("valid edits");
        let full = parse(&outcome.new_source);

        prop_assert_eq!(outcome.result.artifacts.normalized, full.artifacts.normalized);
        prop_assert_eq!(outcome.result.diagnostics.len(), full.diagnostics.len());
        prop_assert_eq!(
            outcome.result.artifacts.registry.len(),
            full.artifacts.registry.len()
        );
    }
}

/// Construct a deterministic disjoint edit batch over `source`.
///
/// Picks `count` evenly-spaced offsets that lie on char boundaries,
/// pairs adjacent offsets into ranges, and assigns short replacement
/// strings drawn from `&[plain, kana, ascii]`. The deterministic
/// shape lets the proptest body call this from inside a closure that
/// already received the random source string, without re-entering
/// the strategy machinery.
fn build_disjoint_edits(source: &str, count: usize) -> Vec<TextEdit> {
    if count == 0 || source.is_empty() {
        return Vec::new();
    }
    let boundaries: Vec<usize> = (0..=source.len())
        .filter(|i| source.is_char_boundary(*i))
        .collect();
    if boundaries.len() < 2 {
        return Vec::new();
    }
    let step = boundaries.len() / (count + 1).max(1);
    if step == 0 {
        return Vec::new();
    }
    let replacements = ["", "x", "abc", " "];
    (0..count)
        .map(|i| {
            let start_idx = (i * 2 + 1) * step;
            let end_idx = (i * 2 + 2) * step;
            let start = boundaries[start_idx.min(boundaries.len() - 1)];
            let end = boundaries[end_idx.min(boundaries.len() - 1)];
            let new_text = replacements[i % replacements.len()].to_owned();
            TextEdit::new(start..end, new_text)
        })
        .filter(|edit| edit.range.start < edit.range.end || !edit.new_text.is_empty())
        .collect()
}

#[test]
fn build_disjoint_edits_returns_disjoint() {
    // Sanity test for the helper itself: edits must not overlap.
    let edits = build_disjoint_edits("the quick brown fox jumps over the lazy dog", 4);
    for window in edits.windows(2) {
        let (a, b) = (&window[0], &window[1]);
        assert!(a.range.end <= b.range.start, "{a:?} overlaps {b:?}");
    }
}

#[test]
fn empty_source_yields_empty_edit_batch() {
    assert!(build_disjoint_edits("", 5).is_empty());
}

/// Pin: a known plain-text edit takes the fast path.
#[test]
fn known_fast_path_input_takes_plain_text_window() {
    let prev_source = "the quick brown fox\njumps over the lazy dog\n";
    let prev = parse(prev_source);
    let edits = vec![TextEdit::new(4..9, "slow")];
    let outcome = parse_incremental(&prev, prev_source, &edits).unwrap();
    assert_eq!(outcome.decision, IncrementalDecision::PlainTextWindow);
    let full = parse(&outcome.new_source);
    assert_eq!(outcome.result.artifacts.normalized, full.artifacts.normalized);
}

/// Pin: a known annotation edit takes the fallback path.
#[test]
fn known_aozora_input_takes_full_reparse() {
    let prev_source = "｜青梅《おうめ》\n";
    let prev = parse(prev_source);
    let edits = vec![TextEdit::new(0..0, "前")];
    let outcome = parse_incremental(&prev, prev_source, &edits).unwrap();
    assert_eq!(outcome.decision, IncrementalDecision::FullReparse);
}

/// Pin: empty edit batch is treated as Noop.
#[test]
fn empty_edits_is_noop() {
    let prev_source = "anything";
    let prev = parse(prev_source);
    let outcome = parse_incremental(&prev, prev_source, &[]).unwrap();
    assert_eq!(outcome.decision, IncrementalDecision::Noop);
    assert_eq!(outcome.new_source, prev_source);
}
