//! Aozora-first lexer — pure-functional pre-pass that extracts every Aozora
//! Bunko construct from source text before the CommonMark parser sees it.
//!
//! Architectural summary:
//!
//! - **No external parser hooks.** The lexer runs first, produces a
//!   normalized text with Private-Use-Area sentinel characters at
//!   Aozora construct positions, plus a side registry mapping sentinel
//!   positions back to pre-classified
//!   `crate::syntax::ast::Node` values.
//! - **Post-process AST walk** substitutes sentinels with the registry's
//!   owned-AST values. That walk lives in `aozora`.
//! - **Pure-functional pipeline**: every stage is `fn(input) -> output`
//!   with no shared mutable state. Unit-testable and deterministic.
//!
//! ## Pipeline (sanitize → tokenize → pair → classify)
//!
//! | Stage | Responsibility |
//! |-------|----------------|
//! | sanitize | BOM strip, CR/LF → LF, PUA collision pre-scan |
//! | tokenize | Linear tokenize — emit trigger events (`｜《》［］※〔〕「」`) |
//! | pair     | Balanced-stack pairing across all delimiters |
//! | classify | Full-spec Aozora classification into `crate::syntax::ast::Node` |
//!
//! After classify, the legacy normalize / registry / validate stages
//! live as a fused walk inside
//! [`crate::pipeline::lex`] — they no longer have standalone stage
//! functions in this crate.
//!
//! ## PUA sentinel scheme
//!
//! Aozora spans are replaced with single characters in the
//! `U+E000..U+F8FF` Private Use Area. Block-level markers become
//! single-character lines so the CommonMark parser treats them as
//! isolated paragraphs that `aozora::post_process` later pairs and
//! collapses.
//!
//! | Sentinel       | Role                                                       |
//! |----------------|------------------------------------------------------------|
//! | [`INLINE_SENTINEL`]     (U+E001) | Inline Aozora span (ruby/bouten/annotation/gaiji/tcy/kaeriten) |
//! | [`BLOCK_LEAF_SENTINEL`] (U+E002) | Block leaf line (page break, section break, leaf indent, sashie) |
//! | [`BLOCK_OPEN_SENTINEL`] (U+E003) | Paired-container open line |
//! | [`BLOCK_CLOSE_SENTINEL`] (U+E004)| Paired-container close line |
//!
//! The sanitize stage pre-scans source for existing PUA usage; any hit
//! triggers a `Diagnostic::SourceContainsPua`.
//!
//! ## Public surface
//!
//! After classify, the lexer module exposes only the per-stage functions
//! used internally by [`crate::pipeline::lex`]. The single result type is
//! the owned, lifetime-free `LexOutput` that [`crate::pipeline::lex`]
//! returns. External
//! direct consumers of this module should be limited to the
//! pipeline driver and benchmarks; everything else goes through
//! [`crate::pipeline::lex`].

// PUA sentinel constants live in `crate::spec` and are re-exported
// here so the `crate::pipeline::lexer::INLINE_SENTINEL` etc. import
// paths inside this crate resolve unchanged.
pub(crate) use crate::spec::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL,
};

pub(crate) mod classify;
pub(crate) mod pair;
pub(crate) mod sanitize;
pub(crate) mod token;
mod tokenize;

pub(crate) use classify::{ClassifiedSpan, SpanKind, classify};
pub(crate) use pair::{PairEvent, pair};
pub(crate) use sanitize::sanitize;
pub(crate) use token::Token;
pub(crate) use tokenize::tokenize;

#[cfg(test)]
mod tests {
    //! Sentinel-constant invariants. Other crate-public surface is
    //! covered by per-stage tests and the lex-path integration tests;
    //! this block keeps the structural invariants that
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
