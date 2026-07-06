//! Aozora notation lex pipeline — owned-AST front door.
//!
//! Both the orchestrator and the per-stage pipeline impl live in
//! this single crate:
//!
//! - The orchestrator (the [`Pipeline`] state machine plus the [`lex`]
//!   entry in `fold`) drives the pipeline through its stages
//!   (sanitize → tokenize → pair → classify). The single public entry
//!   [`lex`] runs the whole thing and returns the result as an owned,
//!   lifetime-free [`LexOutput`] (`Send + Sync`): the classify
//!   stage builds the owned nodes directly into an
//!   `aozora_syntax::ast::NodeStore` (a string interner plus flat
//!   content / segment pools addressed by `u32` handles) — there is no
//!   arena.
//! - The stage implementations live under [`lexer`] (`lexer::sanitize`
//!   through `lexer::classify`). External consumers should reach for
//!   [`lex`] or the [`Pipeline`] state machine; the
//!   per-stage functions are exposed for benchmarks and the
//!   instrumentation feature.
//!
//! [`aozora_scan`] still ships as a separate `no_std` crate — the
//! SIMD trigger scan is independently swappable, benchmarkable, and
//! consumed by `lexer::tokenize` directly.
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
pub mod lexer;
pub mod pipeline;

// Re-export the owned lex output + its source-node / node-ref surface so
// `lex`'s return type is nameable at the crate root (keeps intra-doc links
// resolvable under `-D warnings`) and downstream crates that depend only on
// `aozora-pipeline` (e.g. `aozora-cst`) can name the owned node types.
pub use aozora_syntax::ast::{LexOutput, NodeRef, SourceNode};
pub use fold::lex;
pub use pipeline::{Paired, Pipeline, Sanitized, Source, Tokenized};

/// Eagerly initialise every lazily-built parser table.
///
/// Forces the tokenize-stage SIMD backend choice (`aozora_scan`) and
/// the classify-stage annotation-classifier Aho-Corasick DFA, so the
/// first [`lex`] does not pay the one-time build cost on its
/// critical path. Idempotent and cheap to call repeatedly.
///
/// Lexing stays lazy by default; this is opt-in for latency-sensitive
/// front ends — the umbrella `aozora::prewarm` is the public entry point.
pub fn prewarm() {
    aozora_scan::prewarm();
    lexer::classify::prewarm();
}

/// Re-exports of the sanitize-stage decorative-rule isolator, surfaced
/// so downstream `aozora-render::serialize` can run the same idempotent
/// blank-line-injection pass on its output and converge to a parser
/// fixed point in one cycle. The helpers are otherwise pipeline
/// internals — keep the public surface narrow.
pub use lexer::sanitize::{has_long_rule_line, isolate_decorative_rules};

pub use aozora_spec::{
    ALL_SENTINELS, BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, Diagnostic,
    INLINE_SENTINEL, PairKind, PairLink, SLUGS, Sentinel, SlugEntry, SlugFamily, Span, TriggerKind,
    canonicalise_slug, classify_trigger_bytes,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// `aozora_scan::scan_offsets` MUST yield the exact same byte offsets that
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
        assert_eq!(usize::try_from(out.sanitized_len), Ok("plain text".len()));
    }
}
