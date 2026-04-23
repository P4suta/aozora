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

    /// Phase 6 V1 — a `［＃` digraph survived Phase 4 into the
    /// normalized text. Indicates an annotation escaped classification.
    #[error("residual `［＃` annotation marker in normalized text")]
    #[diagnostic(
        code("afm::lex::residual_annotation_marker"),
        help(
            "a `［＃…］` pair reached the normalizer unclassified — \
             most likely a missing recognizer for the keyword"
        )
    )]
    ResidualAnnotationMarker {
        #[label("leaked here")]
        at: miette::SourceSpan,
        /// Byte-range within the normalized text.
        span: Span,
    },

    /// Phase 6 V2 — a PUA sentinel codepoint was found in the
    /// normalized text at a position that is not recorded in the
    /// placeholder registry. Source-side PUA collisions already
    /// emitted `SourceContainsPua` at Phase 0; a violation here is
    /// distinct: a sentinel landed but the registry does not know
    /// about it, which would break `post_process` splicing.
    #[error("unregistered PUA sentinel {codepoint:?} in normalized text")]
    #[diagnostic(
        code("afm::lex::unregistered_sentinel"),
        help(
            "the normalizer wrote this sentinel but the placeholder registry \
             has no matching entry; post_process cannot resolve it"
        )
    )]
    UnregisteredSentinel {
        #[label("unregistered here")]
        at: miette::SourceSpan,
        codepoint: char,
        /// Byte-range within the normalized text.
        span: Span,
    },

    /// Phase 6 V3 — a placeholder-registry vector is not strictly
    /// ordered by position. Indicates a Phase 4 driver bug.
    #[error("placeholder registry entries are not strictly sorted")]
    #[diagnostic(
        code("afm::lex::registry_out_of_order"),
        help(
            "Phase 4 is expected to emit registry entries in ascending \
             byte-position order; a violation here breaks binary-search lookups"
        )
    )]
    RegistryOutOfOrder {
        #[label("out-of-order pair")]
        at: miette::SourceSpan,
        /// Span covering the two offending entries' positions.
        span: Span,
    },

    /// Phase 6 V3 — a registry entry references a normalized byte
    /// position whose character does not match the expected sentinel
    /// kind. The registry's `inline` vector, for instance, must only
    /// name positions holding `U+E001`.
    #[error("placeholder registry points at {expected:?} but byte there is different")]
    #[diagnostic(
        code("afm::lex::registry_position_mismatch"),
        help(
            "the normalized byte at this position is not the PUA sentinel \
             the registry claims — the registry and the string drifted"
        )
    )]
    RegistryPositionMismatch {
        #[label("mismatch here")]
        at: miette::SourceSpan,
        expected: char,
        /// Byte-range within the normalized text.
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

    /// Quick constructor for [`Diagnostic::ResidualAnnotationMarker`].
    #[must_use]
    pub fn residual_annotation_marker(at: Span) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::ResidualAnnotationMarker {
            at: miette::SourceSpan::new(offset.into(), length),
            span: at,
        }
    }

    /// Quick constructor for [`Diagnostic::UnregisteredSentinel`].
    #[must_use]
    pub fn unregistered_sentinel(at: Span, codepoint: char) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::UnregisteredSentinel {
            at: miette::SourceSpan::new(offset.into(), length),
            codepoint,
            span: at,
        }
    }

    /// Quick constructor for [`Diagnostic::RegistryOutOfOrder`].
    #[must_use]
    pub fn registry_out_of_order(at: Span) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::RegistryOutOfOrder {
            at: miette::SourceSpan::new(offset.into(), length),
            span: at,
        }
    }

    /// Quick constructor for [`Diagnostic::RegistryPositionMismatch`].
    #[must_use]
    pub fn registry_position_mismatch(at: Span, expected: char) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::RegistryPositionMismatch {
            at: miette::SourceSpan::new(offset.into(), length),
            expected,
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

    // ---------------------------------------------------------------
    // Phase 6 diagnostic constructors — cover the V1/V2/V3 shapes so
    // the `#[must_use]` builders and their miette SourceSpan wrapping
    // are exercised by the unit layer (Cov-Ratchet).
    // ---------------------------------------------------------------

    #[test]
    fn residual_annotation_marker_round_trips_span() {
        let diag = Diagnostic::residual_annotation_marker(Span::new(4, 6));
        let Diagnostic::ResidualAnnotationMarker { span, .. } = diag else {
            panic!("expected ResidualAnnotationMarker, got {diag:?}");
        };
        assert_eq!(span, Span::new(4, 6));
    }

    #[test]
    fn residual_annotation_marker_display_mentions_marker() {
        let diag = Diagnostic::residual_annotation_marker(Span::new(0, 2));
        assert!(format!("{diag}").contains("［＃"));
    }

    #[test]
    fn unregistered_sentinel_round_trips_span_and_codepoint() {
        let diag = Diagnostic::unregistered_sentinel(Span::new(1, 4), '\u{E003}');
        let Diagnostic::UnregisteredSentinel {
            codepoint, span, ..
        } = diag
        else {
            panic!("expected UnregisteredSentinel, got {diag:?}");
        };
        assert_eq!(codepoint, '\u{E003}');
        assert_eq!(span, Span::new(1, 4));
    }

    #[test]
    fn unregistered_sentinel_display_mentions_codepoint() {
        let diag = Diagnostic::unregistered_sentinel(Span::new(0, 3), '\u{E004}');
        let rendered = format!("{diag}");
        assert!(
            rendered.contains("E004")
                || rendered.contains("\\u{e004}")
                || rendered.contains('\u{E004}')
        );
    }

    #[test]
    fn registry_out_of_order_round_trips_span() {
        let diag = Diagnostic::registry_out_of_order(Span::new(10, 20));
        let Diagnostic::RegistryOutOfOrder { span, .. } = diag else {
            panic!("expected RegistryOutOfOrder, got {diag:?}");
        };
        assert_eq!(span, Span::new(10, 20));
    }

    #[test]
    fn registry_out_of_order_display_is_descriptive() {
        let diag = Diagnostic::registry_out_of_order(Span::new(0, 5));
        let rendered = format!("{diag}");
        assert!(
            rendered.contains("sort") || rendered.contains("order"),
            "registry out-of-order diagnostic must describe the shape, got {rendered:?}"
        );
    }

    #[test]
    fn registry_position_mismatch_round_trips_span_and_expected() {
        let diag = Diagnostic::registry_position_mismatch(Span::new(2, 5), '\u{E001}');
        let Diagnostic::RegistryPositionMismatch { expected, span, .. } = diag else {
            panic!("expected RegistryPositionMismatch, got {diag:?}");
        };
        assert_eq!(expected, '\u{E001}');
        assert_eq!(span, Span::new(2, 5));
    }

    #[test]
    fn registry_position_mismatch_display_mentions_expected_codepoint() {
        let diag = Diagnostic::registry_position_mismatch(Span::new(0, 1), '\u{E002}');
        let rendered = format!("{diag}");
        assert!(
            rendered.contains("E002")
                || rendered.contains("\\u{e002}")
                || rendered.contains('\u{E002}')
        );
    }
}
