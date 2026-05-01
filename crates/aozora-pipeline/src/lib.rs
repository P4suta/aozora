//! Aozora notation lex pipeline — borrowed-AST front door.
//!
//! Both the orchestrator and the per-phase pipeline impl live in
//! this single crate:
//!
//! - The orchestrator (`pipeline` / `borrowed` modules at the crate
//!   root) drives the borrowed-AST pipeline through its 4 phase
//!   stages. The single public entry [`lex_into_arena`] runs the
//!   whole thing and lands the resulting borrowed AST inside an
//!   `aozora_syntax::borrowed::Arena` provided by the caller.
//! - The phase implementations live under [`lexer`] (`lexer::phase0`
//!   through `lexer::phase3`). External consumers should reach for
//!   [`lex_into_arena`] or the [`Pipeline`] state machine; the
//!   per-phase functions are exposed for benchmarks and the
//!   instrumentation feature.
//!
//! [`aozora_scan`] still ships as a separate `no_std` crate — the
//! SIMD trigger scan is independently swappable, benchmarkable, and
//! consumed by `lexer::phase1` directly.
//!
//! # Observable equivalence
//!
//! [`lex_into_arena`] is a pure function from source text to
//! [`BorrowedLexOutput`] *as observed externally*, even though the
//! internal pipeline mutates the bumpalo arena and runs SIMD scratch
//! buffers. The determinism + sentinel-alignment proptests in
//! `tests/property_borrowed_arena.rs` pin the contract.

#![forbid(unsafe_code)]

mod borrowed;
pub mod lexer;
pub mod pipeline;

pub use aozora_syntax::borrowed::NodeRef;
pub use borrowed::{BorrowedLexOutput, SourceNode, lex_into_arena};
pub use pipeline::{Paired, Pipeline, Sanitized, Source, Tokenized};

pub use aozora_spec::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, Diagnostic, INLINE_SENTINEL,
    PairKind, PairLink, SLUGS, Sentinel, SlugEntry, SlugFamily, Span, TriggerKind,
    canonicalise_slug, classify_trigger_bytes,
};

#[cfg(test)]
mod tests {
    use super::*;
    use aozora_syntax::borrowed::Arena;

    /// `aozora_scan::ScalarScanner` MUST yield the exact same byte
    /// offsets that the legacy phase-1 tokeniser uses for its trigger
    /// positions. We don't have a public hook into phase 1's offsets,
    /// so we cross-check at the [`BorrowedLexOutput`] level: every PUA
    /// sentinel in `normalized` must correspond to a consumed source
    /// trigger.
    #[test]
    fn lex_produces_normalized_with_pua_sentinels_for_trigger_inputs() {
        let arena = Arena::new();
        let out = lex_into_arena("｜青梅《おうめ》", &arena);
        // Exactly one inline sentinel for the ruby span.
        let inline_count = out
            .normalized
            .chars()
            .filter(|c| *c == INLINE_SENTINEL)
            .count();
        assert_eq!(inline_count, 1, "normalized: {:?}", out.normalized);
        assert_eq!(out.registry.count_kind(Sentinel::Inline), 1);
    }

    #[test]
    fn lex_passes_through_plain_text_unchanged() {
        let arena = Arena::new();
        let out = lex_into_arena("hello, world", &arena);
        assert_eq!(out.normalized, "hello, world");
        assert!(out.registry.is_empty());
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn lex_re_exports_sentinel_constants() {
        // Sanity: the constants re-exported from aozora-spec match
        // the values the lexer actually emits, so downstream
        // consumers can use them either via `aozora_lex::*` or
        // `aozora_spec::*` interchangeably.
        assert_eq!(INLINE_SENTINEL, '\u{E001}');
        assert_eq!(BLOCK_LEAF_SENTINEL, '\u{E002}');
        assert_eq!(BLOCK_OPEN_SENTINEL, '\u{E003}');
        assert_eq!(BLOCK_CLOSE_SENTINEL, '\u{E004}');
    }

    #[test]
    fn lex_handles_empty_input() {
        let arena = Arena::new();
        let out = lex_into_arena("", &arena);
        assert!(out.normalized.is_empty());
        assert!(out.registry.is_empty());
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn lex_emits_diagnostics_for_pua_collision() {
        let arena = Arena::new();
        let out = lex_into_arena("abc\u{E001}def", &arena);
        assert!(
            out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::SourceContainsPua { .. })),
            "expected SourceContainsPua, got {:?}",
            out.diagnostics
        );
    }

    #[test]
    fn lex_preserves_sanitized_len_for_segment_merge() {
        // Sanitize is identity on plain text → sanitized_len == source.len().
        let arena = Arena::new();
        let out = lex_into_arena("plain text", &arena);
        assert_eq!(usize::try_from(out.sanitized_len), Ok("plain text".len()));
    }
}
