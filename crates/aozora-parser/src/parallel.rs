//! Intra-document parallel parse via paragraph-level segmentation.
//!
//! The public [`crate::parse`] dispatches into this module when the
//! `parallel` feature is on AND the input crosses
//! [`PARALLEL_THRESHOLD`] AND the segmenter finds more than one
//! independent segment. Otherwise it falls back to the sequential
//! path. The dispatch is transparent to callers: result shape, byte
//! offsets, normalized text, registry, and diagnostics are
//! byte-equivalent to the sequential path. The proptest harness in
//! `tests/property_parallel.rs` pins this invariant on randomly
//! generated aozora-shaped input.
//!
//! # Why this is correct
//!
//! Each phase of [`aozora_lexer::lex`] is locally pure inside a
//! "segment" produced by [`crate::segment::identify_segments`]:
//!
//! - **Phase 0 (sanitize)** is line-local apart from a leading-BOM
//!   strip and inline `〔...〕` accent decomposition; neither
//!   crosses a paragraph break.
//! - **Phase 1 (events) / Phase 2 (pair)**: tokens and bracket
//!   matching are confined to balanced runs; the segmenter keeps any
//!   `［＃ここから…］...［＃ここで…終わり］` paired container in a
//!   single segment.
//! - **Phase 3 (classify) / Phase 4 (emit) / Phase 5 (registry) /
//!   Phase 6 (validate)** are per-token / per-span and have no
//!   cross-paragraph state.
//!
//! Therefore `concat(lex(seg_i)) ≡ lex(concat(seg_i))` after
//! offset-correcting the registry positions and diagnostic spans.
//!
//! # Implementation notes
//!
//! The merge step rebuilds [`PlaceholderRegistry`] in O(N + M) where
//! N is the number of segments and M is the total entry count, by
//! shifting positions in place. Diagnostic spans are shifted by the
//! cumulative *sanitized* length of preceding segments — accent
//! decomposition inside `〔...〕` is the only Phase 0 transform that
//! changes byte length, and we read the per-segment sanitized length
//! from [`aozora_lexer::LexOutput::sanitized_len`].

use core::ops::Range;

use aozora_lexer::{Diagnostic, LexOutput, PlaceholderRegistry, lex};
use aozora_syntax::Span;

#[cfg(feature = "parallel")]
use rayon::prelude::*;

use crate::{ParseArtifacts, ParseResult};
use crate::segment::identify_segments;

/// Parse result for a single document segment, paired with the
/// sanitized-byte length needed for span-correct merging.
///
/// Returned by [`parse_segment`] and consumed by [`merge_segments`].
/// Editor / LSP integrations cache `SegmentParse` per content hash:
/// on the next edit, segments whose content didn't change are
/// retrieved from the cache instead of re-lexed, then merged into a
/// new whole-document `ParseResult` that's byte-equivalent to a
/// fresh `parse()` call.
#[derive(Debug, Clone)]
pub struct SegmentParse {
    /// Per-segment parse output. Diagnostic spans and registry
    /// positions are *local* to the segment (offset 0 = segment
    /// start); callers must shift them via [`merge_segments`] when
    /// composing into a whole-document result.
    pub result: ParseResult,
    /// Phase 0-sanitized byte length of the segment. Required by
    /// [`merge_segments`] to shift diagnostic spans into
    /// whole-document space.
    pub sanitized_len: u32,
}

/// Parse one segment of source text in isolation.
///
/// Equivalent to [`crate::parse_sequential`] except that the result
/// carries [`SegmentParse::sanitized_len`] so the segment can later
/// be merged into a whole-document parse.
#[must_use]
#[tracing::instrument(
    level = "trace",
    skip_all,
    fields(segment_bytes = segment_text.len()),
)]
pub fn parse_segment(segment_text: &str) -> SegmentParse {
    let lex_out = lex(segment_text);
    let sanitized_len = lex_out.sanitized_len;
    SegmentParse {
        result: ParseResult {
            diagnostics: lex_out.diagnostics,
            artifacts: ParseArtifacts {
                normalized: lex_out.normalized,
                registry: lex_out.registry,
            },
        },
        sanitized_len,
    }
}

/// Concatenate per-segment parse results into a single whole-document
/// [`ParseResult`].
///
/// `parts` must be in source order. The output is byte-equivalent to
/// a sequential `parse(concat(segments))` over the same source.
///
/// Use case: an LSP server caches `SegmentParse` per content hash;
/// on every edit it identifies segments, fetches cached entries and
/// freshly parses any that miss the cache, then merges back into a
/// whole-document `ParseResult` in `O(N + M)` where N is segment
/// count and M is total registry entry count.
///
/// # Panics
///
/// Panics if the cumulative normalized or sanitized length of
/// `parts` exceeds `u32::MAX` bytes — the [`Span`] field width
/// throughout the diagnostic path is `u32`. In practice this bound
/// is never hit on aozora-format text (4 GB).
#[must_use]
#[tracing::instrument(
    level = "trace",
    skip_all,
    fields(part_count = parts.len()),
)]
pub fn merge_segments(parts: Vec<SegmentParse>) -> ParseResult {
    let total_normalized: usize = parts.iter().map(|p| p.result.artifacts.normalized.len()).sum();
    let total_inline: usize = parts
        .iter()
        .map(|p| p.result.artifacts.registry.inline.len())
        .sum();
    let total_block_leaf: usize = parts
        .iter()
        .map(|p| p.result.artifacts.registry.block_leaf.len())
        .sum();
    let total_block_open: usize = parts
        .iter()
        .map(|p| p.result.artifacts.registry.block_open.len())
        .sum();
    let total_block_close: usize = parts
        .iter()
        .map(|p| p.result.artifacts.registry.block_close.len())
        .sum();
    let total_diagnostics: usize = parts.iter().map(|p| p.result.diagnostics.len()).sum();

    let mut merged_normalized = String::with_capacity(total_normalized);
    let mut merged_registry = PlaceholderRegistry {
        inline: Vec::with_capacity(total_inline),
        block_leaf: Vec::with_capacity(total_block_leaf),
        block_open: Vec::with_capacity(total_block_open),
        block_close: Vec::with_capacity(total_block_close),
    };
    let mut merged_diagnostics: Vec<Diagnostic> = Vec::with_capacity(total_diagnostics);

    let mut cum_normalized: u32 = 0;
    let mut cum_sanitized: u32 = 0;

    for part in parts {
        merged_normalized.push_str(&part.result.artifacts.normalized);

        for (pos, node) in part.result.artifacts.registry.inline {
            merged_registry.inline.push((pos + cum_normalized, node));
        }
        for (pos, node) in part.result.artifacts.registry.block_leaf {
            merged_registry
                .block_leaf
                .push((pos + cum_normalized, node));
        }
        for (pos, kind) in part.result.artifacts.registry.block_open {
            merged_registry
                .block_open
                .push((pos + cum_normalized, kind));
        }
        for (pos, kind) in part.result.artifacts.registry.block_close {
            merged_registry
                .block_close
                .push((pos + cum_normalized, kind));
        }

        for diag in part.result.diagnostics {
            merged_diagnostics.push(shift_diagnostic_span(diag, cum_sanitized));
        }

        let normalized_delta = u32::try_from(part.result.artifacts.normalized.len())
            .expect("per-segment normalized fits u32");
        cum_normalized = cum_normalized
            .checked_add(normalized_delta)
            .expect("merged normalized length fits u32");
        cum_sanitized = cum_sanitized
            .checked_add(part.sanitized_len)
            .expect("merged sanitized length fits u32");
    }

    ParseResult {
        diagnostics: merged_diagnostics,
        artifacts: ParseArtifacts {
            normalized: merged_normalized,
            registry: merged_registry,
        },
    }
}

/// Inputs ≥ this many bytes go through the parallel path when
/// segmentation produces enough work to amortise the fork-join cost.
///
/// Picked from the `parallel_parse` bench on 16-core x86-64:
///
/// | size  | par vs seq |
/// |-------|------------|
/// | 16 KB | bypassed   |
/// | 64 KB | bypassed   |
/// | 256 KB| ~0.95× (regression — fork overhead dominates) |
/// | 512 KB| ~1.5×      |
/// | 1 MB  | ~2.6×      |
/// | 3 MB  | ~3–4×      |
///
/// 512 KB is the smallest size where the parallel path consistently
/// wins. Conservative so short interactive edits never hit it.
pub const PARALLEL_THRESHOLD: usize = 512 * 1024;

/// Each parallel work item must process at least this many bytes for
/// the per-thread work to amortise rayon's per-task overhead. The
/// segment list is *batched* into groups of ≥ this many bytes before
/// dispatch, so a 1 MB doc with thousands of tiny paragraphs still
/// produces only ~16 parallel work items rather than thousands.
const MIN_PARALLEL_CHUNK_BYTES: usize = 32 * 1024;

/// Run the full parser pipeline with intra-document parallelism.
///
/// Splits the input into independent segments, runs
/// [`aozora_lexer::lex`] on each segment in parallel via rayon, and
/// merges the per-segment outputs into a single byte-equivalent
/// result. Falls back to a sequential single-segment run when only
/// one segment is identified (in which case the parallel path adds
/// no value).
#[cfg(feature = "parallel")]
#[must_use]
#[allow(
    clippy::absolute_paths,
    reason = "tracing::instrument macro takes type paths as field markers"
)]
#[tracing::instrument(
    level = "debug",
    skip_all,
    fields(
        input_bytes = input.len(),
        segment_count = tracing::field::Empty,
        batch_count = tracing::field::Empty,
        threads = tracing::field::Empty,
    ),
)]
pub(crate) fn parse_parallel(input: &str) -> ParseResult {
    let span = tracing::Span::current();
    let segments = identify_segments(input);
    span.record("segment_count", segments.len());
    if segments.len() <= 1 {
        span.record("batch_count", 1u64);
        return parse_sequential_inner(input);
    }
    // Batch consecutive segments so each parallel work item carries
    // ≥ MIN_PARALLEL_CHUNK_BYTES of source. Without this a 1 MB doc
    // with thousands of small paragraphs would spawn thousands of
    // tiny rayon tasks whose fork-join overhead exceeds their lex
    // work. Two consecutive segments are still independently
    // parseable when concatenated (their boundary is a paragraph
    // break and the constituent segments are themselves balanced),
    // so the batched range is itself a valid lex unit.
    let n_threads = rayon::current_num_threads().max(1);
    span.record("threads", n_threads);
    let target_bytes = (input.len() / n_threads).max(MIN_PARALLEL_CHUNK_BYTES);
    let batches = batch_segments(segments, target_bytes);
    span.record("batch_count", batches.len());
    if batches.len() <= 1 {
        return parse_sequential_inner(input);
    }
    let outputs: Vec<(Range<usize>, LexOutput)> = batches
        .into_par_iter()
        .map(|range| (range.clone(), lex(&input[range])))
        .collect();
    debug_assert!(
        !outputs.is_empty(),
        "rayon par_iter must produce at least one output (we filtered batches.len() <= 1 above)"
    );
    merge_lex_outputs(outputs)
}

/// Collapse consecutive segments into batches of byte length
/// ≥ `target_bytes`. The last batch may be shorter. Each input
/// segment is a half-open `Range<usize>`; consecutive segments
/// always abut (no gaps), so the merged range is well-defined.
fn batch_segments(segments: Vec<Range<usize>>, target_bytes: usize) -> Vec<Range<usize>> {
    if segments.is_empty() {
        return Vec::new();
    }
    let mut batched = Vec::with_capacity(segments.len());
    let mut start = segments[0].start;
    let mut end = segments[0].end;
    for seg in segments.into_iter().skip(1) {
        if end - start >= target_bytes {
            batched.push(start..end);
            start = seg.start;
        }
        end = seg.end;
    }
    batched.push(start..end);
    batched
}

/// Sequential reference path. Always available — the parallel
/// dispatch falls back here for short inputs and single-segment
/// inputs, and tests use it as the byte-equivalence reference for
/// `parse()`.
///
/// Public so the property-test suite (and downstream consumers that
/// want to opt out of parallelism without disabling the `parallel`
/// feature) can call it directly. Internally identical to a
/// `parse()` that bypasses the threshold check.
#[must_use]
pub fn parse_sequential(input: &str) -> ParseResult {
    parse_sequential_inner(input)
}

/// Sequential fallback used by:
///
/// - non-`parallel` builds (the only entry point),
/// - `parallel` builds where the input is below
///   [`PARALLEL_THRESHOLD`] or yields only one segment.
#[must_use]
pub(crate) fn parse_sequential_inner(input: &str) -> ParseResult {
    let lex_out = lex(input);
    ParseResult {
        diagnostics: lex_out.diagnostics,
        artifacts: ParseArtifacts {
            normalized: lex_out.normalized,
            registry: lex_out.registry,
        },
    }
}

/// Concatenate per-segment [`LexOutput`]s with offset correction.
///
/// `outputs` is a list of `(source_range, lex_output)` tuples in
/// segment order. Returns a single [`ParseResult`] equivalent to a
/// hypothetical `parse(concat(source_segments))`. Returns
/// `ParseResult` rather than [`LexOutput`] because [`LexOutput`] is
/// `#[non_exhaustive]` upstream, so it can't be literal-constructed
/// outside `aozora-lexer`; bypassing the intermediate sidesteps the
/// constructor-or-not question.
fn merge_lex_outputs(outputs: Vec<(Range<usize>, LexOutput)>) -> ParseResult {
    // Pre-size: every per-segment Vec contributes its full length to
    // the merged output. Avoids reallocations on multi-MB inputs.
    let total_normalized: usize = outputs.iter().map(|(_, o)| o.normalized.len()).sum();
    let total_inline: usize = outputs.iter().map(|(_, o)| o.registry.inline.len()).sum();
    let total_block_leaf: usize = outputs.iter().map(|(_, o)| o.registry.block_leaf.len()).sum();
    let total_block_open: usize = outputs.iter().map(|(_, o)| o.registry.block_open.len()).sum();
    let total_block_close: usize = outputs.iter().map(|(_, o)| o.registry.block_close.len()).sum();
    let total_diagnostics: usize = outputs.iter().map(|(_, o)| o.diagnostics.len()).sum();

    let mut merged_normalized = String::with_capacity(total_normalized);
    let mut merged_registry = PlaceholderRegistry {
        inline: Vec::with_capacity(total_inline),
        block_leaf: Vec::with_capacity(total_block_leaf),
        block_open: Vec::with_capacity(total_block_open),
        block_close: Vec::with_capacity(total_block_close),
    };
    let mut merged_diagnostics: Vec<Diagnostic> = Vec::with_capacity(total_diagnostics);

    let mut cum_normalized: u32 = 0;
    let mut cum_sanitized: u32 = 0;

    for (_range, lex_out) in outputs {
        // 1. Normalized text concatenation.
        merged_normalized.push_str(&lex_out.normalized);

        // 2. Registry: shift each (pos, _) by cum_normalized.
        for (pos, node) in lex_out.registry.inline {
            merged_registry.inline.push((pos + cum_normalized, node));
        }
        for (pos, node) in lex_out.registry.block_leaf {
            merged_registry
                .block_leaf
                .push((pos + cum_normalized, node));
        }
        for (pos, kind) in lex_out.registry.block_open {
            merged_registry
                .block_open
                .push((pos + cum_normalized, kind));
        }
        for (pos, kind) in lex_out.registry.block_close {
            merged_registry
                .block_close
                .push((pos + cum_normalized, kind));
        }

        // 3. Diagnostics: shift every span by cum_sanitized.
        for diag in lex_out.diagnostics {
            merged_diagnostics.push(shift_diagnostic_span(diag, cum_sanitized));
        }

        // 4. Advance cumulative offsets for the next segment.
        let normalized_delta = u32::try_from(lex_out.normalized.len())
            .expect("per-segment normalized fits u32 (LexOutput already pinned via Span::u32)");
        cum_normalized = cum_normalized
            .checked_add(normalized_delta)
            .expect("merged normalized length fits u32");
        cum_sanitized = cum_sanitized
            .checked_add(lex_out.sanitized_len)
            .expect("merged sanitized length fits u32");
    }

    ParseResult {
        diagnostics: merged_diagnostics,
        artifacts: ParseArtifacts {
            normalized: merged_normalized,
            registry: merged_registry,
        },
    }
}

/// Shift every [`Span`] embedded in `diag` by `delta` bytes.
///
/// [`Diagnostic`] carries both a `span: Span` (programmatic offsets)
/// and an `at: miette::SourceSpan` (renderer offsets) per variant.
/// We rebuild via the public constructors so both fields stay in
/// lockstep — a hand-rolled struct literal would silently leave
/// `at` un-shifted.
///
/// `Diagnostic` is `#[non_exhaustive]` upstream; the trailing
/// catch-all documents the contract that future variants must pick
/// up shifts here.
fn shift_diagnostic_span(diag: Diagnostic, delta: u32) -> Diagnostic {
    match diag {
        Diagnostic::SourceContainsPua { span, codepoint, .. } => {
            Diagnostic::source_contains_pua(shift_span(span, delta), codepoint)
        }
        Diagnostic::UnclosedBracket { span, kind, .. } => {
            Diagnostic::unclosed_bracket(shift_span(span, delta), kind)
        }
        Diagnostic::UnmatchedClose { span, kind, .. } => {
            Diagnostic::unmatched_close(shift_span(span, delta), kind)
        }
        Diagnostic::ResidualAnnotationMarker { span, .. } => {
            Diagnostic::residual_annotation_marker(shift_span(span, delta))
        }
        Diagnostic::UnregisteredSentinel { span, codepoint, .. } => {
            Diagnostic::unregistered_sentinel(shift_span(span, delta), codepoint)
        }
        Diagnostic::RegistryOutOfOrder { span, .. } => {
            Diagnostic::registry_out_of_order(shift_span(span, delta))
        }
        Diagnostic::RegistryPositionMismatch { span, expected, .. } => {
            Diagnostic::registry_position_mismatch(shift_span(span, delta), expected)
        }
        // `#[non_exhaustive]`: a future variant lands here. The
        // unshifted diag passes through as-is; merge correctness for
        // that variant becomes a follow-up.
        other => other,
    }
}

#[inline]
fn shift_span(span: Span, delta: u32) -> Span {
    Span::new(span.start + delta, span.end + delta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    /// Sequential and parallel paths must produce identical
    /// normalized text on inputs the parallel path can split.
    #[test]
    fn parallel_matches_sequential_on_simple_paragraphs() {
        let input = "first paragraph\n\nsecond paragraph\n\nthird paragraph";
        let seq = parse_sequential_inner(input);
        #[cfg(feature = "parallel")]
        {
            let par = parse_parallel(input);
            assert_eq!(par.artifacts.normalized, seq.artifacts.normalized);
            assert_eq!(par.diagnostics.len(), seq.diagnostics.len());
            assert_eq!(
                par.artifacts.registry.inline.len(),
                seq.artifacts.registry.inline.len()
            );
        }
    }

    #[test]
    fn parallel_matches_sequential_with_inline_ruby_per_paragraph() {
        let input = "｜漢字《かんじ》\n\nplain\n\n｜日本《にほん》";
        let seq = parse_sequential_inner(input);
        #[cfg(feature = "parallel")]
        {
            let par = parse_parallel(input);
            assert_eq!(par.artifacts.normalized, seq.artifacts.normalized);
            assert_eq!(
                par.artifacts.registry.inline.len(),
                seq.artifacts.registry.inline.len()
            );
            // Verify offsets are shifted, not duplicated.
            let positions: Vec<u32> =
                par.artifacts.registry.inline.iter().map(|(p, _)| *p).collect();
            for window in positions.windows(2) {
                assert!(window[0] < window[1], "registry not strictly sorted: {positions:?}");
            }
        }
    }

    #[test]
    fn parallel_keeps_paired_container_in_one_segment() {
        let input = "［＃ここから割書］\n\ninner1\n\ninner2\n\n［＃ここで割書終わり］";
        let seq = parse_sequential_inner(input);
        #[cfg(feature = "parallel")]
        {
            let par = parse_parallel(input);
            assert_eq!(par.artifacts.normalized, seq.artifacts.normalized);
            assert_eq!(
                par.artifacts.registry.block_open.len(),
                seq.artifacts.registry.block_open.len()
            );
            assert_eq!(
                par.artifacts.registry.block_close.len(),
                seq.artifacts.registry.block_close.len()
            );
        }
    }

    #[test]
    fn dispatch_falls_back_for_short_input() {
        // Below threshold → public parse() goes sequential. We just
        // verify the public API still produces the right shape.
        let out = parse("short");
        assert_eq!(out.artifacts.normalized, "short");
    }

    #[test]
    fn dispatch_handles_single_segment_input() {
        // Long single-paragraph input: identify_segments returns one
        // segment so parse_parallel falls back to the sequential
        // inner path. Ensure correctness.
        let mut buf = String::with_capacity(100_000);
        for _ in 0..2000 {
            buf.push_str("the quick brown fox\n");
        }
        let out = parse(&buf);
        assert_eq!(out.artifacts.normalized.len(), buf.len());
    }

    #[test]
    fn shift_span_is_additive() {
        let s = Span::new(10, 20);
        let shifted = shift_span(s, 5);
        assert_eq!(shifted.start, 15);
        assert_eq!(shifted.end, 25);
    }

    // -----------------------------------------------------------
    // Public segment API: parse_segment + merge_segments
    // -----------------------------------------------------------

    #[test]
    fn parse_segment_roundtrip_matches_full_parse() {
        let s = "first\n\nsecond\n\nthird";
        let segments = identify_segments(s);
        let parts: Vec<SegmentParse> = segments
            .iter()
            .map(|r| parse_segment(&s[r.clone()]))
            .collect();
        let merged = merge_segments(parts);
        let full = parse_sequential_inner(s);
        assert_eq!(merged.artifacts.normalized, full.artifacts.normalized);
        assert_eq!(merged.diagnostics.len(), full.diagnostics.len());
    }

    #[test]
    fn merge_segments_handles_inline_ruby_per_segment() {
        let s = "｜青梅《おうめ》\n\n｜日本《にほん》";
        let segments = identify_segments(s);
        let parts: Vec<SegmentParse> = segments
            .iter()
            .map(|r| parse_segment(&s[r.clone()]))
            .collect();
        let merged = merge_segments(parts);
        let full = parse_sequential_inner(s);
        assert_eq!(merged.artifacts.normalized, full.artifacts.normalized);
        assert_eq!(
            merged.artifacts.registry.inline.len(),
            full.artifacts.registry.inline.len()
        );
        // Positions must be monotonically increasing in the merged
        // registry (the "sorted-by-construction" invariant).
        for window in merged.artifacts.registry.inline.windows(2) {
            assert!(window[0].0 < window[1].0, "registry not sorted");
        }
    }

    #[test]
    fn parse_segment_carries_sanitized_len() {
        let p = parse_segment("hello");
        assert_eq!(p.sanitized_len, 5);
    }

    #[test]
    fn merge_segments_empty_returns_empty_result() {
        let merged = merge_segments(Vec::new());
        assert!(merged.artifacts.normalized.is_empty());
        assert!(merged.diagnostics.is_empty());
        assert_eq!(merged.artifacts.registry.inline.len(), 0);
    }
}
