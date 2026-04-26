//! Aozora Bunko notation parser (legacy owned-AST API).
//!
//! As of I-2.2, the canonical user-facing API lives in the [`aozora`]
//! meta crate (`Document` + `AozoraTree`) and uses the borrowed AST
//! exclusively. This crate is now a thin wrapper around the legacy
//! owned-AST entries (`aozora_lexer::lex`, `aozora_lex::lex` →
//! [`PlaceholderRegistry`]) that the parallel and incremental
//! orchestrators still consume internally.
//!
//! # Public surface (post I-2.2 deletion)
//!
//! - [`parse`] — runs the lexer and packages the owned-shape result
//!   into [`ParseResult`]. Used by [`incremental`] and the parallel
//!   re-export in `aozora-parallel`.
//! - [`incremental`] — TextEdit + apply_edits + parse_incremental
//!   (still owned-AST under the hood).
//! - [`parallel`] / [`segment`] — paragraph-segmented parallel parse.
//!   Re-exported by `aozora-parallel`.
//!
//! Removed in I-2.2 Commit E (functionality migrated to
//! [`aozora-render`] which consumes borrowed AST directly):
//!
//! - `html::*` — HTML rendering. Replaced by [`aozora_render::html`].
//! - `serialize::*` — Source-text inverse. Replaced by
//!   [`aozora_render::serialize`].
//! - `test_support::*` — Test helpers. Lived only for this crate's
//!   integration tests, all of which are themselves removed.
//! - `aozora::{html, bouten, classes}` — Per-node renderer mirror;
//!   replaced by [`aozora_render::render_node`] + [`aozora_render::bouten`].
//!
//! # Architectural invariant
//!
//! ADR-0001 (zero parser hooks): all Aozora recognition lives in the
//! lexer; this crate only consumes the lexer's normalized text + registry.

#![forbid(unsafe_code)]

pub mod incremental;
pub mod parallel;
pub mod segment;

pub use aozora_lex::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, Diagnostic, INLINE_SENTINEL,
    PlaceholderRegistry, lex,
};
// `aozora_lexer` is still re-exported as `lexer` for backward
// compatibility with downstream consumers that reach into the legacy
// phase modules.
pub use aozora_lexer as lexer;
pub use incremental::{
    EditError, IncrementalDecision, IncrementalOutcome, TextEdit, apply_edits, parse_incremental,
};
pub use parallel::{SegmentParse, merge_segments, parse_segment, parse_sequential};
pub use segment::identify_segments;

/// Output of [`parse`]: the lexer diagnostics and the
/// [`ParseArtifacts`] needed by downstream consumers.
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
    /// Lexer-side artifacts: normalized text + placeholder registry.
    pub artifacts: ParseArtifacts,
}

/// Inputs needed by downstream consumers — the PUA-sentinel-normalized
/// text and the placeholder registry that maps every sentinel position
/// back to the originating [`aozora_syntax::owned::AozoraNode`] /
/// [`aozora_syntax::ContainerKind`].
#[derive(Debug, Clone)]
pub struct ParseArtifacts {
    /// Normalized source text: the original Aozora input with every
    /// recognised span replaced by a PUA sentinel.
    pub normalized: String,
    /// Sentinel-position → originating `AozoraNode` / `ContainerKind`
    /// lookup.
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
/// to the sequential path.
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
}
