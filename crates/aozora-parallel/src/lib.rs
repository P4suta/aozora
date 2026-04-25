//! Intra-document segment-parallel parse orchestrator.
//!
//! This crate is the public namespace for paragraph-segment-based
//! parallel parsing of Aozora documents. The implementation is
//! currently re-exported from `aozora-parser`'s legacy
//! `parallel`/`segment` modules; once Move 3's façade settles and
//! the fused engine in `aozora-lex` (Move 2 follow-up) takes over,
//! the implementation physically migrates here and the
//! `aozora-parser` dependency drops.
//!
//! ## Public surface
//!
//! - [`identify_segments`] — partition source into independently
//!   lex-able byte ranges (paragraph-delimited, paired-container
//!   aware).
//! - [`parse_segment`] — lex one segment in isolation.
//! - [`merge_segments`] — fold per-segment outputs into a single
//!   whole-document parse result.
//! - [`parse_sequential`] — sequential reference path (used by the
//!   parallel dispatcher's fallback).
//! - [`SegmentParse`] — per-segment cache entry.
//! - [`PARALLEL_THRESHOLD`] — input-size threshold above which the
//!   public `parse()` dispatches to the parallel path.

#![forbid(unsafe_code)]

pub use aozora_parser::parallel::{
    PARALLEL_THRESHOLD, SegmentParse, merge_segments, parse_segment, parse_sequential,
};
pub use aozora_parser::segment::identify_segments;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identify_segments_returns_at_least_one_range_for_non_empty_input() {
        let segments = identify_segments("paragraph one\n\nparagraph two");
        assert!(!segments.is_empty());
    }

    #[test]
    fn parallel_threshold_is_at_least_one_kb() {
        // Sanity: the threshold is bench-driven and should never sink
        // to a value where short interactive edits hit the parallel
        // path. Using a `const` assert at runtime so a future drop
        // below the floor surfaces in test output even if some
        // upstream change makes the constant smaller.
        const _: () = assert!(PARALLEL_THRESHOLD >= 1024);
    }
}
