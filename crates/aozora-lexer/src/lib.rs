//! Aozora-first lexer — pure-functional pre-pass that extracts every Aozora
//! Bunko construct from source text before the CommonMark parser sees it.
//!
//! See ADR-0008 for the architectural rationale. In summary:
//!
//! - **No parser hooks in the upstream CommonMark parser**. The lexer runs
//!   first, produces a normalized text with Private-Use-Area sentinel
//!   characters at Aozora construct positions, plus a side registry mapping
//!   sentinel positions back to pre-classified
//!   [`aozora_syntax::borrowed::AozoraNode`] values. The CommonMark parser
//!   sees only plain CommonMark + GFM.
//! - **Post-comrak AST walk** substitutes sentinels with the registry's
//!   borrowed-AST values. That walk lives in `afm-parser`.
//! - **Pure-functional pipeline**: every phase is `fn(input) -> output` with
//!   no shared mutable state. Unit-testable and deterministic.
//!
//! ## Pipeline (4 phases)
//!
//! | Phase | Responsibility |
//! |-------|----------------|
//! | 0 sanitize | BOM strip, CR/LF → LF, PUA collision pre-scan |
//! | 1 events   | Linear tokenize — emit trigger events (`｜《》［］※〔〕「」`) |
//! | 2 pair     | Balanced-stack pairing across all delimiters |
//! | 3 classify | Full-spec Aozora classification into [`borrowed::AozoraNode`] |
//!
//! After F.3, the legacy phases 4 (normalize) / 5 (registry) /
//! 6 (validate) live as a fused walk inside
//! [`aozora_lex::lex_into_arena`][lex_into_arena] — they no longer have
//! standalone phase functions in this crate.
//!
//! [borrowed::AozoraNode]: aozora_syntax::borrowed::AozoraNode
//! [lex_into_arena]: ../aozora_lex/fn.lex_into_arena.html
//!
//! ## PUA sentinel scheme
//!
//! Aozora spans are replaced with single characters in the [`U+E000..U+F8FF`]
//! Private Use Area. Block-level markers become single-character lines so
//! the CommonMark parser treats them as isolated paragraphs that
//! `afm-parser::post_process` later pairs and collapses.
//!
//! | Sentinel       | Role                                                       |
//! |----------------|------------------------------------------------------------|
//! | [`INLINE_SENTINEL`]     (U+E001) | Inline Aozora span (ruby/bouten/annotation/gaiji/tcy/kaeriten) |
//! | [`BLOCK_LEAF_SENTINEL`] (U+E002) | Block leaf line (page break, section break, leaf indent, sashie) |
//! | [`BLOCK_OPEN_SENTINEL`] (U+E003) | Paired-container open line |
//! | [`BLOCK_CLOSE_SENTINEL`] (U+E004)| Paired-container close line |
//!
//! Phase 0 pre-scans source for existing PUA usage; any hit triggers a
//! `Diagnostic::SourceContainsPua`.
//!
//! ## Public surface
//!
//! After F.3, `aozora-lexer` is a build-block crate exposing only the
//! per-phase functions used internally by `aozora-lex`. The "package
//! result" (the legacy `LexOutput`) is replaced by
//! [`aozora_lex::lex_into_arena`]'s `BorrowedLexOutput<'a>`. External
//! direct consumers of this crate should be limited to `aozora-lex` and
//! benchmarks; everything else goes through `aozora-lex`.

#![forbid(unsafe_code)]

// PUA sentinel constants moved to `aozora-spec`. Re-exported here so
// the existing `aozora_lexer::INLINE_SENTINEL` etc. import paths keep
// working through the 0.1 → 0.2 transition (Move 1.2 compatibility
// shim).
pub use aozora_spec::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL,
};

pub mod diagnostic;
#[cfg(feature = "phase3-instrument")]
pub mod instrumentation;
mod phase0_sanitize;
mod phase1_events;
pub mod phase2_pair;
pub mod phase3_classify;
pub mod token;

pub use diagnostic::Diagnostic;
pub use phase0_sanitize::{SanitizeOutput, sanitize};
#[doc(hidden)]
pub use phase0_sanitize::{
    has_long_rule_line, isolate_decorative_rules, normalize_line_endings, rewrite_accent_spans,
    scan_for_sentinel_collisions,
};
pub use phase1_events::{Tokenizer, tokenize, tokenize_in};
pub use phase2_pair::{
    EventTag, PairEvent, PairEventStream, PairKind, PairOutputIn, PairStream, pair, pair_in,
};
pub use phase3_classify::{ClassifiedSpan, ClassifyStream, SpanKind, classify};
pub use token::{Token, TokenStream, TokenTag, TriggerKind};

#[cfg(test)]
mod tests {
    //! Sentinel-constant invariants. Other crate-public surface is
    //! covered by per-phase tests and the borrowed-pipeline tests in
    //! `aozora-lex`; this block keeps the structural invariants that
    //! every downstream consumer relies on (PUA range membership +
    //! pairwise distinctness) co-located with the re-exports.
    use super::*;

    #[test]
    fn sentinel_constants_are_in_pua_range() {
        for &c in &[
            INLINE_SENTINEL,
            BLOCK_LEAF_SENTINEL,
            BLOCK_OPEN_SENTINEL,
            BLOCK_CLOSE_SENTINEL,
        ] {
            let code = u32::from(c);
            assert!(
                (0xE000..=0xF8FF).contains(&code),
                "{c:?} ({code:#06X}) must lie in Unicode PUA"
            );
        }
    }

    #[test]
    fn sentinel_constants_are_distinct() {
        let sentinels = [
            INLINE_SENTINEL,
            BLOCK_LEAF_SENTINEL,
            BLOCK_OPEN_SENTINEL,
            BLOCK_CLOSE_SENTINEL,
        ];
        for (i, a) in sentinels.iter().enumerate() {
            for b in &sentinels[i + 1..] {
                assert_ne!(a, b, "sentinels must be pairwise distinct");
            }
        }
    }
}
