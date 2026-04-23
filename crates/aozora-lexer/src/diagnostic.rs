//! Diagnostic stream produced by the lexer.
//!
//! A [`Diagnostic`] is a non-fatal observation about the source: the lexer
//! always produces a best-effort [`crate::LexOutput`] and never aborts mid-
//! stream. Callers decide how to surface the diagnostics — the CLI can
//! render them via [`miette::Report`], tests can assert on the variants,
//! library consumers can ignore them.
//!
//! Variants are added as phases land. The current enumeration covers
//! Phase 0 (source sanitation); Phases 2–4 add their own as they land.

use afm_syntax::Span;
use miette::Diagnostic as MietteDiagnostic;
use thiserror::Error;

/// Observation emitted by any lexer phase.
///
/// Every variant carries a byte-range [`Span`] in the *original source*
/// (pre-normalization), so miette's snippet renderer points at the right
/// characters regardless of which phase detected the issue.
#[derive(Debug, Clone, Error, MietteDiagnostic)]
#[non_exhaustive]
pub enum Diagnostic {
    /// Source contains a codepoint that collides with one of the lexer's
    /// PUA sentinel reservations ([`crate::INLINE_SENTINEL`],
    /// [`crate::BLOCK_LEAF_SENTINEL`], [`crate::BLOCK_OPEN_SENTINEL`],
    /// [`crate::BLOCK_CLOSE_SENTINEL`]). Downstream phases will emit those
    /// same codepoints into normalized text, so the collision means the
    /// placeholder registry can no longer distinguish source-text
    /// occurrences from lexer-inserted markers.
    ///
    /// Current behavior: emit this diagnostic and proceed. A future
    /// enhancement can fall back to Unicode noncharacters
    /// (`U+FDD0..U+FDEF`) for the sentinel set when a collision is
    /// detected.
    #[error("source contains lexer PUA sentinel codepoint {codepoint:?}")]
    #[diagnostic(
        code("afm::lex::source_contains_pua"),
        help(
            "the lexer reserves U+E001..U+E004 as inline/block markers; \
             a source-side occurrence will confuse the placeholder registry"
        )
    )]
    SourceContainsPua {
        #[label("here")]
        at: miette::SourceSpan,
        codepoint: char,
        /// Byte-range in the original source for programmatic consumers
        /// that don't need miette's [`miette::SourceSpan`].
        span: Span,
    },
}

impl Diagnostic {
    /// Quick constructor for [`Diagnostic::SourceContainsPua`]. Converts
    /// the caller's [`Span`] into a miette [`miette::SourceSpan`] so the
    /// variant's `at` field does not need callers to duplicate the
    /// offsets.
    #[must_use]
    pub fn source_contains_pua(at: Span, codepoint: char) -> Self {
        let offset = at.start as usize;
        let length = (at.end - at.start) as usize;
        Self::SourceContainsPua {
            at: miette::SourceSpan::new(offset.into(), length),
            codepoint,
            span: at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_contains_pua_round_trips_span() {
        let diag = Diagnostic::source_contains_pua(Span::new(5, 8), '\u{E001}');
        match diag {
            Diagnostic::SourceContainsPua {
                codepoint, span, ..
            } => {
                assert_eq!(codepoint, '\u{E001}');
                assert_eq!(span, Span::new(5, 8));
            }
        }
    }

    #[test]
    fn source_contains_pua_display_mentions_codepoint() {
        let diag = Diagnostic::source_contains_pua(Span::new(0, 3), '\u{E002}');
        let rendered = format!("{diag}");
        assert!(
            rendered.contains("E002")
                || rendered.contains("\\u{e002}")
                || rendered.contains('\u{E002}')
        );
    }
}
