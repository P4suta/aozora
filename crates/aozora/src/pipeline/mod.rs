//! Aozora notation lex pipeline — owned-AST front door.
//!
//! Both the orchestrator and the per-stage pipeline impl live in
//! this single crate:
//!
//! - The orchestrator (the `Pipeline` state machine plus the [`lex`]
//!   entry in `fold`) drives the pipeline through its stages
//!   (sanitize → tokenize → pair → classify). The single public entry
//!   [`lex`] runs the whole thing and returns the result as an owned,
//!   lifetime-free [`LexOutput`] (`Send + Sync`): the classify
//!   stage builds the owned nodes directly into an
//!   `crate::syntax::ast::NodeStore` (a string interner plus flat
//!   content / segment pools addressed by `u32` handles) — there is no
//!   arena.
//! - The stage implementations live under [`lexer`] (`lexer::sanitize`
//!   through `lexer::classify`). External consumers should reach for
//!   [`lex`] or the `Pipeline` state machine; the
//!   per-stage functions are exposed for benchmarks and the
//!   instrumentation feature.
//!
//! The SIMD trigger scan lives in [`crate::scan`] — independently
//! swappable and benchmarkable, consumed by `lexer::tokenize` directly.
//!
//! # Observable equivalence
//!
//! [`lex`] is a pure function from source text to
//! [`LexOutput`] *as observed externally*, even though the
//! internal pipeline runs SIMD trigger scans over scratch buffers.
//! The determinism + sentinel-alignment proptests in
//! `tests/property_owned_output.rs` pin the contract.

#![forbid(unsafe_code)]

mod fold;
pub(crate) mod lexer;
pub(crate) mod state_machine;

use crate::scan;
pub(crate) use crate::syntax::ast::{LexOutput, SourceNode};
pub(crate) use fold::lex;

/// Eagerly initialise every lazily-built parser table.
///
/// Forces the tokenize-stage SIMD backend choice ([`crate::scan`]) and
/// the classify-stage annotation-classifier Aho-Corasick DFA, so the
/// first [`lex`] does not pay the one-time build cost on its
/// critical path. Idempotent and cheap to call repeatedly.
///
/// Lexing stays lazy by default; this is opt-in for latency-sensitive
/// front ends — the umbrella `aozora::prewarm` is the public entry point.
pub(crate) fn prewarm() {
    scan::prewarm();
    lexer::classify::prewarm();
}

/// The sanitize-stage decorative-rule isolator, surfaced so `crate::render`'s
/// serialize path can run the same idempotent blank-line-injection pass on its
/// output and converge to a parser fixed point in one cycle.
pub(crate) use lexer::sanitize::{has_long_rule_line, isolate_decorative_rules};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{
        BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, Diagnostic,
        INLINE_SENTINEL, Sentinel,
    };

    /// `crate::scan::scan_offsets` MUST yield the exact same byte offsets that
    /// the tokenize-stage tokeniser uses for its trigger positions. We
    /// cross-check at the [`LexOutput`] level: every PUA sentinel in
    /// `normalized` must correspond to a consumed source trigger.
    #[test]
    fn lex_produces_normalized_with_pua_sentinels_for_trigger_inputs() {
        let out = lex("｜青梅《おうめ》");
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
        let out = lex("hello, world");
        assert_eq!(out.normalized, "hello, world");
        assert!(out.registry.is_empty());
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn lex_re_exports_sentinel_constants() {
        assert_eq!(INLINE_SENTINEL, '\u{E001}');
        assert_eq!(BLOCK_LEAF_SENTINEL, '\u{E002}');
        assert_eq!(BLOCK_OPEN_SENTINEL, '\u{E003}');
        assert_eq!(BLOCK_CLOSE_SENTINEL, '\u{E004}');
    }

    #[test]
    fn lex_handles_empty_input() {
        let out = lex("");
        assert!(out.normalized.is_empty());
        assert!(out.registry.is_empty());
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn lex_emits_diagnostics_for_pua_collision() {
        let out = lex("abc\u{E001}def");
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
        let out = lex("plain text");
        assert_eq!(out.sanitized.len(), "plain text".len());
    }
}
