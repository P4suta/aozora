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

use crate::phase2_pair::PairKind;

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

    /// An open delimiter reached end-of-input with no matching close on
    /// the pairing stack. Phase 2 marks the orphan open as
    /// [`crate::phase2_pair::PairEvent::Unclosed`] and keeps going so the
    /// rest of the document still parses; Phase 3 decides, per-kind,
    /// whether the malformed span still has recoverable text.
    #[error("unclosed Aozora {kind:?} bracket")]
    #[diagnostic(
        code("afm::lex::unclosed_bracket"),
        help(
            "the opener has no matching close delimiter — either the close \
             was omitted or an earlier close matched a nested opener"
        )
    )]
    UnclosedBracket {
        #[label("opened here")]
        at: miette::SourceSpan,
        kind: PairKind,
        /// Byte-range of the unmatched *open* delimiter in the sanitized
        /// source.
        span: Span,
    },

    /// A close delimiter was seen with an empty stack, or with a stack
    /// top of a different [`PairKind`]. The phase records the close as
    /// [`crate::phase2_pair::PairEvent::Unmatched`] but does *not* pop
    /// the stack, so a later, correctly-matching close can still pair
    /// with the original open.
    #[error("unmatched Aozora {kind:?} close delimiter")]
    #[diagnostic(
        code("afm::lex::unmatched_close"),
        help(
            "no matching open on the pairing stack — either the open was \
             omitted or an inner unmatched close consumed it"
        )
    )]
    UnmatchedClose {
        #[label("close here")]
        at: miette::SourceSpan,
        kind: PairKind,
        /// Byte-range of the stray *close* delimiter.
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
        let (offset, length) = span_to_miette_parts(at);
        Self::SourceContainsPua {
            at: miette::SourceSpan::new(offset.into(), length),
            codepoint,
            span: at,
        }
    }

    /// Quick constructor for [`Diagnostic::UnclosedBracket`].
    #[must_use]
    pub fn unclosed_bracket(at: Span, kind: PairKind) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::UnclosedBracket {
            at: miette::SourceSpan::new(offset.into(), length),
            kind,
            span: at,
        }
    }

    /// Quick constructor for [`Diagnostic::UnmatchedClose`].
    #[must_use]
    pub fn unmatched_close(at: Span, kind: PairKind) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::UnmatchedClose {
            at: miette::SourceSpan::new(offset.into(), length),
            kind,
            span: at,
        }
    }
}

/// Split an afm [`Span`] into the `(offset, length)` pair miette wants.
///
/// Centralizing this avoids repeating the `u32 → usize` cast at every
/// diagnostic constructor and keeps the `Span → SourceSpan` contract in
/// one place.
const fn span_to_miette_parts(span: Span) -> (usize, usize) {
    let offset = span.start as usize;
    let length = (span.end - span.start) as usize;
    (offset, length)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_contains_pua_round_trips_span() {
        let diag = Diagnostic::source_contains_pua(Span::new(5, 8), '\u{E001}');
        let Diagnostic::SourceContainsPua {
            codepoint, span, ..
        } = diag
        else {
            panic!("expected SourceContainsPua, got {diag:?}");
        };
        assert_eq!(codepoint, '\u{E001}');
        assert_eq!(span, Span::new(5, 8));
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

    #[test]
    fn unclosed_bracket_round_trips_span_and_kind() {
        let diag = Diagnostic::unclosed_bracket(Span::new(3, 6), PairKind::Bracket);
        match diag {
            Diagnostic::UnclosedBracket { kind, span, .. } => {
                assert_eq!(kind, PairKind::Bracket);
                assert_eq!(span, Span::new(3, 6));
            }
            other => panic!("expected UnclosedBracket, got {other:?}"),
        }
    }

    #[test]
    fn unmatched_close_round_trips_span_and_kind() {
        let diag = Diagnostic::unmatched_close(Span::new(7, 10), PairKind::Ruby);
        match diag {
            Diagnostic::UnmatchedClose { kind, span, .. } => {
                assert_eq!(kind, PairKind::Ruby);
                assert_eq!(span, Span::new(7, 10));
            }
            other => panic!("expected UnmatchedClose, got {other:?}"),
        }
    }

    #[test]
    fn unclosed_bracket_display_mentions_kind() {
        let diag = Diagnostic::unclosed_bracket(Span::new(0, 3), PairKind::Tortoise);
        assert!(format!("{diag}").contains("Tortoise"));
    }

    #[test]
    fn unmatched_close_display_mentions_kind() {
        let diag = Diagnostic::unmatched_close(Span::new(0, 3), PairKind::Quote);
        assert!(format!("{diag}").contains("Quote"));
    }
}
