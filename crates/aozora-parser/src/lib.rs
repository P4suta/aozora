//! Aozora Bunko notation parser.
//!
//! Wraps the pure-functional [`aozora_lexer`] pipeline into a single
//! [`parse`] entry point and pairs it with a registry-driven
//! [`serialize`] inverse and a generic HTML rendering surface
//! ([`html::render_to_string`]).
//!
//! # Layered API surface
//!
//! - [`parse`] — run the lexer over a UTF-8 source, returning a
//!   [`ParseResult`] carrying the lexer diagnostics and the
//!   [`ParseArtifacts`] (`normalized` text + `registry`) needed by
//!   [`serialize`] / [`html::render_to_string`].
//! - [`serialize`] — invert the pipeline, emitting Aozora source text
//!   from a [`ParseResult`] via registry-driven PUA-sentinel
//!   substitution (`O(normalized.len())`, fixed-point after one
//!   round-trip; see ADR-0005 corpus sweep I3).
//! - [`html::render_to_string`] — render parsed text to semantic HTML5,
//!   using the per-node renderer in [`aozora::html::render`] for
//!   inline/block Aozora nodes and a thin block-level walker for
//!   paragraph / hard-break / container nesting.
//! - [`aozora::html::render`] — per-node renderer with a generic
//!   `&mut dyn core::fmt::Write` writer; the integration point for
//!   downstream consumers that embed [`aozora_syntax::AozoraNode`]
//!   into their own document tree (see sibling repo `afm` for the
//!   CommonMark+GFM Markdown-dialect example).
//!
//! # Architectural invariant
//!
//! ADR-0001 (zero parser hooks): all Aozora recognition lives in the
//! lexer; the parser only consumes the lexer's normalized text + registry.
//! No state is hidden behind reactive callbacks; every phase is a pure
//! function from one shape to the next.

#![forbid(unsafe_code)]

pub mod aozora;
pub mod html;
pub mod incremental;
pub mod parallel;
pub mod segment;
pub mod serialize;

#[doc(hidden)]
pub mod test_support;

pub use aozora_lex::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, Diagnostic, INLINE_SENTINEL,
    PlaceholderRegistry, lex,
};
// `aozora_lexer` is still re-exported as `lexer` for backward
// compatibility with downstream consumers that reach into the legacy
// phase modules. Move 2's fused engine will absorb those modules and
// this re-export will deprecate.
pub use aozora_lexer as lexer;
pub use incremental::{
    EditError, IncrementalDecision, IncrementalOutcome, TextEdit, apply_edits, parse_incremental,
};
pub use parallel::{SegmentParse, merge_segments, parse_segment, parse_sequential};
pub use segment::identify_segments;
pub use serialize::{serialize, serialize_from_artifacts};

/// Output of [`parse`]: the lexer diagnostics and the
/// [`ParseArtifacts`] needed to invert the pipeline.
///
/// `diagnostics` is always present; it is `Vec::new()` when the
/// lexer found nothing to complain about. Consumers that want a
/// pass/fail decision (a `--strict` CLI flag, a Language-Server
/// integration, a corpus sweep) can inspect `diagnostics.is_empty()`
/// without having to rerun the lexer.
#[derive(Debug, Clone)]
pub struct ParseResult {
    /// Non-fatal observations from the lexer (unclosed opens, stray
    /// triggers, PUA collisions, …). Empty on the happy path.
    pub diagnostics: Vec<Diagnostic>,
    /// Lexer-side artifacts needed by [`serialize`] and
    /// [`html::render_to_string`].
    pub artifacts: ParseArtifacts,
}

/// Inputs to [`serialize`] / [`html::render_to_string`] that the
/// lexer computed during [`parse`].
///
/// Holds the PUA-sentinel-normalized text and the placeholder
/// registry that maps every sentinel position back to the originating
/// [`aozora_syntax::AozoraNode`] / [`aozora_syntax::ContainerKind`].
#[derive(Debug, Clone)]
pub struct ParseArtifacts {
    /// Normalized source text: the original Aozora input with every
    /// recognised span replaced by a PUA sentinel.
    pub normalized: String,
    /// Sentinel-position → originating `AozoraNode` / `ContainerKind`
    /// lookup. Consumed by [`serialize`] /
    /// [`html::render_to_string`] via the registry's
    /// `inline_at` / `block_leaf_at` / `block_open_at` /
    /// `block_close_at` binary-search accessors.
    pub registry: PlaceholderRegistry,
}

/// Parse a UTF-8 Aozora-notation source buffer.
///
/// Runs [`aozora_lexer::lex`] and packages the result into a
/// [`ParseResult`]. The lexer is the only Aozora recogniser in the
/// pipeline (ADR-0001).
///
/// # Parallelism
///
/// With the `parallel` feature on (default), inputs above
/// [`parallel::PARALLEL_THRESHOLD`] are dispatched to a paragraph-
/// segmented parallel path that runs `lex` over independent segments
/// in a rayon thread pool, then merges the per-segment outputs. The
/// dispatch is transparent: result shape, byte offsets, normalized
/// text, registry positions, and diagnostic spans are byte-equivalent
/// to the sequential path. Inputs below the threshold or producing a
/// single segment go through the sequential path with zero rayon
/// overhead.
#[must_use]
pub fn parse(input: &str) -> ParseResult {
    #[cfg(feature = "parallel")]
    {
        if input.len() >= parallel::PARALLEL_THRESHOLD {
            return parallel::parse_parallel(input);
        }
    }
    parallel::parse_sequential_inner(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn parses_plain_paragraph_yields_no_diagnostics() {
        let result = parse("Hello, world.");
        assert!(result.diagnostics.is_empty());
        assert_eq!(result.artifacts.normalized, "Hello, world.");
    }

    #[test]
    fn ruby_with_explicit_delimiter_normalises_to_one_inline_sentinel() {
        let result = parse("｜青梅《おうめ》へ");
        let inline_count = result
            .artifacts
            .normalized
            .chars()
            .filter(|c| *c == INLINE_SENTINEL)
            .count();
        assert_eq!(
            inline_count, 1,
            "expected 1 inline sentinel, got {} (normalized: {:?})",
            inline_count, result.artifacts.normalized
        );
    }

    #[test]
    fn round_trip_is_fixed_point_for_canonical_ruby() {
        let src = "｜青梅《おうめ》";
        let first = serialize(&parse(src));
        let second = serialize(&parse(&first));
        assert_eq!(
            first, second,
            "serialize ∘ parse must be a fixed point after one round-trip"
        );
    }
}
