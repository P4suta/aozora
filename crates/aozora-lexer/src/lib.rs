//! Aozora-first lexer — pure-functional pre-pass that extracts every Aozora
//! Bunko construct from source text before the CommonMark parser sees it.
//!
//! See ADR-0008 for the architectural rationale. In summary:
//!
//! - **No parser hooks in the upstream CommonMark parser**. The lexer runs
//!   first, produces a normalized text with Private-Use-Area sentinel
//!   characters at Aozora construct positions, plus a side registry mapping
//!   sentinel positions back to pre-classified [`afm_syntax::AozoraNode`]
//!   values. The CommonMark parser sees only plain CommonMark + GFM.
//! - **Post-comrak AST walk** substitutes sentinels with the registry's
//!   [`afm_syntax::AozoraNode`] values. That walk lives in `afm-parser`.
//! - **Pure-functional pipeline**: every phase is `fn(input) -> output` with
//!   no shared mutable state. Unit-testable and deterministic.
//!
//! ## Pipeline (7 phases)
//!
//! | Phase | Responsibility |
//! |-------|----------------|
//! | 0 sanitize | BOM strip, CR/LF → LF, PUA collision pre-scan |
//! | 1 events   | Linear tokenize — emit trigger events (`｜《》［］※〔〕「」`) |
//! | 2 pair     | Balanced-stack pairing across all delimiters |
//! | 3 classify | Full-spec Aozora classification into `AozoraNode` |
//! | 4 normalize| Text rewrite: accent decompose + gaiji → UCS + Aozora → PUA sentinels |
//! | 5 registry | Sorted placeholder registry for O(log N) lookup |
//! | 6 validate | Assert invariants V1-V4 (sentinel integrity, registry coverage) |
//!
//! The public entry point is [`lex`], which chains the 7 phases into a
//! single [`LexOutput`].
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
//! `Diagnostic::SourceContainsPua`. A later enhancement can fall back to
//! Unicode noncharacters (`U+FDD0..U+FDEF`, reserved by Unicode for
//! application internal use and never assigned) if collision becomes a
//! recurring issue.
//!
//! ## Status
//!
//! This crate is newly scaffolded (commit A3 in the plan). Phase
//! implementations land incrementally in commits C1-C7. Until then the
//! public surface is the type declarations below and [`lex`] returns a
//! placeholder [`LexOutput`] wrapping the source verbatim — a stub that
//! lets downstream consumers (afm-parser integration) land in parallel
//! without blocking on lexer completion.

#![forbid(unsafe_code)]

/// Private-Use-Area character reserved as the inline Aozora placeholder
/// in normalized text. See module docs.
pub const INLINE_SENTINEL: char = '\u{E001}';
/// Private-Use-Area character reserved as the block-leaf line sentinel.
pub const BLOCK_LEAF_SENTINEL: char = '\u{E002}';
/// Private-Use-Area character reserved as the paired-container open line sentinel.
pub const BLOCK_OPEN_SENTINEL: char = '\u{E003}';
/// Private-Use-Area character reserved as the paired-container close line sentinel.
pub const BLOCK_CLOSE_SENTINEL: char = '\u{E004}';

pub mod diagnostic;
mod phase0_sanitize;
mod phase1_events;
pub mod phase2_pair;
pub mod phase3_classify;
pub mod phase4_normalize;
mod phase5_registry;
pub mod token;
// Phase-implementation modules land in subsequent commits C7.
// mod phase6_validate;
// pub mod source_map;

pub use diagnostic::Diagnostic;
pub use phase0_sanitize::{SanitizeOutput, sanitize};
pub use phase1_events::tokenize;
pub use phase2_pair::{PairEvent, PairKind, PairOutput, pair};
pub use phase3_classify::{ClassifiedSpan, ClassifyOutput, SpanKind, classify};
pub use phase4_normalize::{NormalizeOutput, PlaceholderRegistry, normalize};
pub use token::{Token, TriggerKind};

/// Placeholder output shape.
///
/// Replaced with the full [`LexOutput`] (normalized text + registry +
/// source map + diagnostics) as phases land. Exposed now so `afm-parser`
/// can start integrating against the API seam (commit E1).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LexOutput {
    /// Raw source text passed through unchanged. Replaced with the
    /// normalized (sentinel-bearing) text in Phase 4.
    pub normalized: String,
}

/// Lex `source` into a [`LexOutput`]. Pure function; no I/O, no global state.
///
/// Currently a stub that returns the source verbatim. Full implementation
/// arrives in commits C1-C7.
#[must_use]
pub fn lex(source: &str) -> LexOutput {
    LexOutput {
        normalized: source.to_owned(),
    }
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn lex_stub_returns_source_verbatim() {
        let input = "plain text with ｜漢字《かんじ》 ruby";
        let out = lex(input);
        assert_eq!(out.normalized, input);
    }

    #[test]
    fn lex_stub_handles_empty_input() {
        let out = lex("");
        assert!(out.normalized.is_empty());
    }
}
