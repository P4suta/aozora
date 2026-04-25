//! Aozora-first lexer — pure-functional pre-pass that extracts every Aozora
//! Bunko construct from source text before the CommonMark parser sees it.
//!
//! See ADR-0008 for the architectural rationale. In summary:
//!
//! - **No parser hooks in the upstream CommonMark parser**. The lexer runs
//!   first, produces a normalized text with Private-Use-Area sentinel
//!   characters at Aozora construct positions, plus a side registry mapping
//!   sentinel positions back to pre-classified [`aozora_syntax::AozoraNode`]
//!   values. The CommonMark parser sees only plain CommonMark + GFM.
//! - **Post-comrak AST walk** substitutes sentinels with the registry's
//!   [`aozora_syntax::AozoraNode`] values. That walk lives in `afm-parser`.
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
//! ## Shape at a glance
//!
//! [`lex`] runs each phase once, in order, and returns a
//! [`LexOutput`] packaging the normalized text, placeholder registry,
//! and accumulated diagnostics. Each phase is a pure function of its
//! inputs; re-running the pipeline on the same source yields
//! byte-identical output (verified by the determinism property test
//! in `afm-parser`'s post-process invariants).

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
pub mod phase6_validate;
pub mod token;
// A `SourceMap` module (normalized position → source byte offset)
// would slot in here if future milestones need it.

pub use diagnostic::Diagnostic;
pub use phase0_sanitize::{SanitizeOutput, sanitize};
pub use phase1_events::tokenize;
pub use phase2_pair::{PairEvent, PairKind, PairOutput, pair};
pub use phase3_classify::{ClassifiedSpan, ClassifyOutput, SpanKind, classify};
pub use phase4_normalize::{NormalizeOutput, PlaceholderRegistry, normalize};
pub use phase6_validate::{ValidateOutput, validate};
pub use token::{Token, TriggerKind};

/// Public output of the lexer pipeline.
///
/// Contains the normalized text (which the comrak parser consumes
/// verbatim), the placeholder registry (which `afm-parser`'s
/// `post_process` uses to splice Aozora constructs back into the AST),
/// and the accumulated diagnostics from every phase.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LexOutput {
    /// Normalized text with every Aozora construct replaced by a PUA
    /// sentinel. Safe to pass to a vanilla CommonMark/GFM parser.
    pub normalized: String,
    /// Sentinel-position → original classification lookup tables.
    pub registry: PlaceholderRegistry,
    /// Non-fatal observations accumulated across Phase 0..6.
    pub diagnostics: Vec<Diagnostic>,
}

/// Run the lexer pipeline over `source`.
///
/// Pure function; no I/O, no global state. Chains Phases 0..6:
///
/// 1. [`sanitize`] — BOM strip, CRLF→LF, PUA collision scan.
/// 2. [`tokenize`] — linear trigger-event extraction.
/// 3. [`pair`] — balanced-stack pair cross-linking.
/// 4. [`classify`] — per-span `AozoraNode` classification.
/// 5. [`normalize`] — PUA sentinel substitution + registry build.
/// 6. [`validate`] — V1..V3 structural invariants.
///
/// Accent decomposition happens inside [`sanitize`]; gaiji UCS
/// resolution happens during [`classify`] via
/// `aozora_encoding::gaiji::lookup`. `SourceMap` construction is not
/// yet layered in and would fold into this entrypoint without
/// changing the shape of [`LexOutput`].
#[must_use]
pub fn lex(source: &str) -> LexOutput {
    let sanitized = sanitize(source);
    let tokens = tokenize(&sanitized.text);
    let pair_out = pair(&tokens);
    let classify_out = classify(&pair_out, &sanitized.text);
    let mut normalize_out = normalize(&classify_out, &sanitized.text);
    // Merge Phase 0 diagnostics into the accumulator; Phase 0 output
    // is only otherwise reachable via `sanitize()`'s direct return.
    let mut diagnostics = sanitized.diagnostics;
    diagnostics.append(&mut normalize_out.diagnostics);
    normalize_out.diagnostics = diagnostics;

    let validated = validate(normalize_out);
    LexOutput {
        normalized: validated.normalized,
        registry: validated.registry,
        diagnostics: validated.diagnostics,
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
    fn lex_plain_text_passes_through_unchanged() {
        let input = "plain text only";
        let out = lex(input);
        assert_eq!(out.normalized, input);
        assert!(out.registry.is_empty());
    }

    #[test]
    fn lex_inline_ruby_becomes_single_sentinel() {
        let input = "｜漢字《かんじ》";
        let out = lex(input);
        assert_eq!(out.normalized, "\u{E001}");
        assert_eq!(out.registry.inline.len(), 1);
    }

    #[test]
    fn lex_surfaces_phase0_pua_diagnostic() {
        let input = "abc\u{E001}def";
        let out = lex(input);
        assert!(
            out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::SourceContainsPua { .. })),
            "expected SourceContainsPua forwarded into LexOutput, got {:?}",
            out.diagnostics
        );
    }

    #[test]
    fn lex_stub_handles_empty_input() {
        let out = lex("");
        assert!(out.normalized.is_empty());
    }
}
