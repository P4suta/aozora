//! Lexer-emitted observations.
//!
//! A [`Diagnostic`] is non-fatal: the lexer always produces a
//! best-effort output and never aborts mid-stream. Callers decide how
//! to surface the diagnostics — the CLI can render them via
//! [`miette::Report`], tests can assert on the variants, library
//! consumers can ignore them.
//!
//! Every variant carries a byte-range [`Span`] in the *original source*
//! (pre-normalization), so miette's snippet renderer points at the
//! right characters regardless of which phase detected the issue.
//!
//! # Severity and source axes
//!
//! Diagnostics split along two orthogonal axes:
//!
//! - **[`Severity`]** — `Error` / `Warning` / `Note`. Determines how
//!   strictly a host (CLI, LSP, editor decorator) should treat the
//!   observation. Defaults to `Error` for genuine syntax issues and
//!   `Warning` for input that the parser can carry around but the
//!   user should be told about.
//! - **[`DiagnosticSource`]** — `Source` (problem traces back to
//!   user input) vs. `Internal` (a pipeline-invariant violation;
//!   appearance indicates a library bug). Hosts that filter by
//!   `Internal` get a clear "library bug" channel without having to
//!   match on individual variants.
//!
//! Each variant exposes both axes through accessors
//! ([`Diagnostic::severity`] / [`Diagnostic::source`]).
//!
//! # Internal variant
//!
//! The four library-bug sanity checks
//! (`ResidualAnnotationMarker`, `UnregisteredSentinel`,
//! `RegistryOutOfOrder`, `RegistryPositionMismatch`) live as a single
//! [`Diagnostic::Internal`] variant whose `code` field
//! ([`InternalCheckCode`]) tags the specific check. Tests and tooling
//! match on that code via [`codes`]. Consumers that want to filter
//! library-bug diagnostics out of the [`crate::Diagnostic`] stream
//! reach for [`Diagnostic::source`].

use miette::Diagnostic as MietteDiagnostic;
use thiserror::Error;

use crate::PairKind;
use crate::Span;

/// Stable identifier strings for known [`Diagnostic`] variants.
///
/// [`Diagnostic::code`] returns one of these for any production
/// diagnostic. They are guaranteed stable across patch and minor
/// releases; major-release variant additions land new constants here
/// without touching existing ones.
pub mod codes {
    /// Source contains a lexer PUA sentinel codepoint.
    pub const SOURCE_CONTAINS_PUA: &str = "aozora::lex::source_contains_pua";

    /// Open delimiter reached end-of-input with no matching close.
    pub const UNCLOSED_BRACKET: &str = "aozora::lex::unclosed_bracket";

    /// Close delimiter saw an empty stack or a mismatched stack top.
    pub const UNMATCHED_CLOSE: &str = "aozora::lex::unmatched_close";

    /// Pipeline-internal: an `［＃` digraph survived classification
    /// into the normalized text. Indicates a missing recogniser for
    /// the keyword.
    pub const RESIDUAL_ANNOTATION_MARKER: &str = "aozora::lex::residual_annotation_marker";

    /// Pipeline-internal: a PUA sentinel codepoint is present in the
    /// normalized text at a position that is not recorded in the
    /// placeholder registry.
    ///
    /// Source-side PUA collisions emit [`SOURCE_CONTAINS_PUA`]
    /// upstream; this code is distinct.
    pub const UNREGISTERED_SENTINEL: &str = "aozora::lex::unregistered_sentinel";

    /// Pipeline-internal: a placeholder-registry vector is not
    /// strictly ordered by position. Indicates a normalizer driver
    /// bug.
    pub const REGISTRY_OUT_OF_ORDER: &str = "aozora::lex::registry_out_of_order";

    /// Pipeline-internal: a registry entry references a normalized
    /// byte position whose character does not match the expected
    /// sentinel kind.
    pub const REGISTRY_POSITION_MISMATCH: &str = "aozora::lex::registry_position_mismatch";
}

/// Severity of a [`Diagnostic`].
///
/// Hosts route diagnostics by severity: `Error` blocks downstream
/// rendering or fails CI, `Warning` decorates the editor surface,
/// `Note` is informational. The `aozora` library never panics on a
/// `Diagnostic` — the parser produces a best-effort output and
/// surfaces this enum as the host's policy hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Severity {
    /// Genuine error; downstream consumers should treat the parse as
    /// suspect.
    Error,
    /// Recoverable observation; parse continues and output is
    /// preserved, but the user should know.
    Warning,
    /// Informational note; editor surfaces may show it as a tooltip
    /// or annotation but it does not affect CI / build status.
    Note,
}

impl Severity {
    /// Every variant in declaration order. Used by codegen so
    /// downstream artefacts track the enum without drift.
    pub const ALL: [Self; 3] = [Self::Error, Self::Warning, Self::Note];

    /// Stable lowercase wire-format identifier ("error" / "warning"
    /// / "note"). The same string the driver wire format emits in
    /// the `severity` field of `DiagnosticWire`.
    #[must_use]
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Note => "note",
        }
    }
}

/// Origin of a [`Diagnostic`] — distinguishes user-input issues from
/// library-internal sanity-check failures.
///
/// Production parses on well-formed input never emit `Internal`
/// diagnostics. An `Internal` diagnostic indicates a bug in
/// `aozora-pipeline` and SHOULD be reported upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiagnosticSource {
    /// Issue traces back to the user-provided source text.
    Source,
    /// Pipeline-internal invariant violation. Indicates a library
    /// bug; the parse is still completed best-effort but downstream
    /// tooling should surface this distinctly.
    Internal,
}

impl DiagnosticSource {
    /// Every variant in declaration order.
    pub const ALL: [Self; 2] = [Self::Source, Self::Internal];

    /// Stable lowercase wire-format identifier ("source" /
    /// "internal"). Matches the `source` field of `DiagnosticWire`.
    #[must_use]
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Internal => "internal",
        }
    }
}

/// Identifier of a specific pipeline-internal sanity check.
///
/// Carried by the [`Diagnostic::Internal`] variant. Tooling that
/// wants per-check assertions matches on this enum; legacy callers
/// (logs, regex grep) can still reach for the stable
/// `aozora::lex::*` string via [`Self::as_code`].
///
/// `#[non_exhaustive]` so adding a new check variant is a minor
/// release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InternalCheckCode {
    /// An `［＃` digraph survived classification into the normalized
    /// text. Indicates a missing recogniser for the keyword.
    ResidualAnnotationMarker,
    /// A PUA sentinel codepoint is present in the normalized text at
    /// a position that is not recorded in the placeholder registry.
    UnregisteredSentinel,
    /// A placeholder-registry vector is not strictly ordered by
    /// position. Indicates a normalizer driver bug.
    RegistryOutOfOrder,
    /// A registry entry references a normalized byte position whose
    /// character does not match the expected sentinel kind.
    RegistryPositionMismatch,
}

impl InternalCheckCode {
    /// All known internal check codes in declaration order.
    pub const ALL: [Self; 4] = [
        Self::ResidualAnnotationMarker,
        Self::UnregisteredSentinel,
        Self::RegistryOutOfOrder,
        Self::RegistryPositionMismatch,
    ];

    /// Stable `aozora::lex::*` string identifier for this check.
    /// Equivalent to the corresponding [`codes`] constant.
    #[must_use]
    pub const fn as_code(self) -> &'static str {
        match self {
            Self::ResidualAnnotationMarker => codes::RESIDUAL_ANNOTATION_MARKER,
            Self::UnregisteredSentinel => codes::UNREGISTERED_SENTINEL,
            Self::RegistryOutOfOrder => codes::REGISTRY_OUT_OF_ORDER,
            Self::RegistryPositionMismatch => codes::REGISTRY_POSITION_MISMATCH,
        }
    }
}

/// Observation emitted by any lexer phase.
#[derive(Debug, Clone, Error, MietteDiagnostic)]
#[non_exhaustive]
pub enum Diagnostic {
    /// Source contains a codepoint that collides with one of the
    /// lexer's PUA sentinel reservations
    /// ([`crate::INLINE_SENTINEL`], [`crate::BLOCK_LEAF_SENTINEL`],
    /// [`crate::BLOCK_OPEN_SENTINEL`], [`crate::BLOCK_CLOSE_SENTINEL`]).
    /// Downstream phases will emit those same codepoints into normalized
    /// text, so a collision means the placeholder registry can no longer
    /// distinguish source-text occurrences from lexer-inserted markers.
    #[error("source contains lexer PUA sentinel codepoint {codepoint:?}")]
    #[diagnostic(
        code("aozora::lex::source_contains_pua"),
        severity(Warning),
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
    /// the pairing stack.
    #[error("unclosed Aozora {kind:?} bracket")]
    #[diagnostic(
        code("aozora::lex::unclosed_bracket"),
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
    /// top of a different [`PairKind`].
    #[error("unmatched Aozora {kind:?} close delimiter")]
    #[diagnostic(
        code("aozora::lex::unmatched_close"),
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

    /// Pipeline-internal sanity-check failure — production parses on
    /// well-formed input never emit this. The [`check`](Self::Internal)
    /// payload identifies the specific check via the typed
    /// [`InternalCheckCode`] enum; tooling that prefers the stable
    /// string identifier reaches via
    /// [`Self::code`](Self::code). Library consumers that just want
    /// to filter "library bugs" out of the stream check
    /// [`source`](Self::source) instead.
    #[error("internal aozora pipeline check failed: {}", check.as_code())]
    #[diagnostic(
        code("aozora::internal"),
        help(
            "this is a pipeline-internal sanity check; appearance \
             indicates a bug in aozora — please report at \
             https://github.com/P4suta/aozora/issues with the source \
             that triggered it"
        )
    )]
    Internal {
        #[label("at this position")]
        at: miette::SourceSpan,
        /// Typed identifier for the specific check that fired. Pin
        /// per-check assertions on this rather than the stringly-typed
        /// [`code`](Self::code) accessor so the compiler enforces
        /// match exhaustiveness at the call site.
        check: InternalCheckCode,
        /// Byte-range covering the violation site.
        span: Span,
    },
}

#[allow(
    clippy::same_name_method,
    reason = "intentional: our inherent severity() / code() return strongly-typed (Severity enum, &'static str) values that mirror miette::Diagnostic's loosely-typed defaults — callers prefer the inherent method"
)]
impl Diagnostic {
    /// Constructor for [`Diagnostic::SourceContainsPua`].
    #[must_use]
    pub fn source_contains_pua(at: Span, codepoint: char) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::SourceContainsPua {
            at: miette::SourceSpan::new(offset.into(), length),
            codepoint,
            span: at,
        }
    }

    /// Constructor for [`Diagnostic::UnclosedBracket`].
    #[must_use]
    pub fn unclosed_bracket(at: Span, kind: PairKind) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::UnclosedBracket {
            at: miette::SourceSpan::new(offset.into(), length),
            kind,
            span: at,
        }
    }

    /// Constructor for [`Diagnostic::UnmatchedClose`].
    #[must_use]
    pub fn unmatched_close(at: Span, kind: PairKind) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::UnmatchedClose {
            at: miette::SourceSpan::new(offset.into(), length),
            kind,
            span: at,
        }
    }

    /// Constructor for [`Diagnostic::Internal`]. Takes a typed
    /// [`InternalCheckCode`] — the compiler enforces that every
    /// production emit-site classifies the check correctly.
    #[must_use]
    pub fn internal(at: Span, check: InternalCheckCode) -> Self {
        let (offset, length) = span_to_miette_parts(at);
        Self::Internal {
            at: miette::SourceSpan::new(offset.into(), length),
            check,
            span: at,
        }
    }

    /// Severity routing axis. See [`Severity`].
    ///
    /// `#[non_exhaustive]` puts the responsibility on every match
    /// here for adding-new-variant time, not on a catch-all arm —
    /// the compiler will refuse to build until the new variant is
    /// classified.
    #[must_use]
    pub fn severity(&self) -> Severity {
        match self {
            Self::SourceContainsPua { .. } => Severity::Warning,
            Self::UnclosedBracket { .. } | Self::UnmatchedClose { .. } | Self::Internal { .. } => {
                Severity::Error
            }
        }
    }

    /// Origin axis: user input vs. pipeline-internal. See
    /// [`DiagnosticSource`].
    #[must_use]
    pub fn source(&self) -> DiagnosticSource {
        match self {
            Self::SourceContainsPua { .. }
            | Self::UnclosedBracket { .. }
            | Self::UnmatchedClose { .. } => DiagnosticSource::Source,
            Self::Internal { .. } => DiagnosticSource::Internal,
        }
    }

    /// Byte-range covering the diagnostic.
    #[must_use]
    pub fn span(&self) -> Span {
        match self {
            Self::SourceContainsPua { span, .. }
            | Self::UnclosedBracket { span, .. }
            | Self::UnmatchedClose { span, .. }
            | Self::Internal { span, .. } => *span,
        }
    }

    /// Stable string identifier for this diagnostic. Returns one of
    /// the constants from [`codes`] for production variants, or the
    /// `Internal` payload's [`InternalCheckCode::as_code`] for
    /// pipeline-internal checks.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::SourceContainsPua { .. } => codes::SOURCE_CONTAINS_PUA,
            Self::UnclosedBracket { .. } => codes::UNCLOSED_BRACKET,
            Self::UnmatchedClose { .. } => codes::UNMATCHED_CLOSE,
            Self::Internal { check, .. } => check.as_code(),
        }
    }
}

/// Split a [`Span`] into the `(offset, length)` pair miette wants.
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
    fn source_contains_pua_is_warning_severity() {
        let diag = Diagnostic::source_contains_pua(Span::new(0, 3), '\u{E002}');
        assert_eq!(diag.severity(), Severity::Warning);
        assert_eq!(diag.source(), DiagnosticSource::Source);
        assert_eq!(diag.code(), codes::SOURCE_CONTAINS_PUA);
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
    fn unclosed_bracket_is_error_severity_from_source() {
        let diag = Diagnostic::unclosed_bracket(Span::new(0, 3), PairKind::Bracket);
        assert_eq!(diag.severity(), Severity::Error);
        assert_eq!(diag.source(), DiagnosticSource::Source);
        assert_eq!(diag.code(), codes::UNCLOSED_BRACKET);
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
    fn unmatched_close_is_error_severity_from_source() {
        let diag = Diagnostic::unmatched_close(Span::new(0, 3), PairKind::Quote);
        assert_eq!(diag.severity(), Severity::Error);
        assert_eq!(diag.source(), DiagnosticSource::Source);
        assert_eq!(diag.code(), codes::UNMATCHED_CLOSE);
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

    #[test]
    fn internal_round_trips_check_and_span() {
        let diag = Diagnostic::internal(Span::new(2, 5), InternalCheckCode::RegistryOutOfOrder);
        let Diagnostic::Internal { check, span, .. } = diag else {
            panic!("expected Internal, got {diag:?}");
        };
        assert_eq!(check, InternalCheckCode::RegistryOutOfOrder);
        assert_eq!(span, Span::new(2, 5));
    }

    #[test]
    fn internal_classified_as_internal_source() {
        let diag = Diagnostic::internal(Span::new(0, 1), InternalCheckCode::UnregisteredSentinel);
        assert_eq!(diag.severity(), Severity::Error);
        assert_eq!(diag.source(), DiagnosticSource::Internal);
        assert_eq!(diag.code(), codes::UNREGISTERED_SENTINEL);
    }

    #[test]
    fn internal_display_mentions_code() {
        let diag =
            Diagnostic::internal(Span::new(0, 1), InternalCheckCode::ResidualAnnotationMarker);
        let rendered = format!("{diag}");
        assert!(
            rendered.contains(codes::RESIDUAL_ANNOTATION_MARKER),
            "Internal Display should print the code; got {rendered:?}"
        );
    }

    #[test]
    fn internal_check_code_as_code_round_trips_constants() {
        for kind in InternalCheckCode::ALL {
            let diag = Diagnostic::internal(Span::new(0, 0), kind);
            assert_eq!(
                diag.code(),
                kind.as_code(),
                "code() must agree with as_code() for {kind:?}"
            );
        }
    }

    /// Codes are stable identifiers — pin every constant so accidental
    /// rename of one breaks this test rather than silently breaking
    /// downstream tooling that grep-matches on the string.
    #[test]
    fn code_constants_are_stable() {
        assert_eq!(
            codes::SOURCE_CONTAINS_PUA,
            "aozora::lex::source_contains_pua"
        );
        assert_eq!(codes::UNCLOSED_BRACKET, "aozora::lex::unclosed_bracket");
        assert_eq!(codes::UNMATCHED_CLOSE, "aozora::lex::unmatched_close");
        assert_eq!(
            codes::RESIDUAL_ANNOTATION_MARKER,
            "aozora::lex::residual_annotation_marker"
        );
        assert_eq!(
            codes::UNREGISTERED_SENTINEL,
            "aozora::lex::unregistered_sentinel"
        );
        assert_eq!(
            codes::REGISTRY_OUT_OF_ORDER,
            "aozora::lex::registry_out_of_order"
        );
        assert_eq!(
            codes::REGISTRY_POSITION_MISMATCH,
            "aozora::lex::registry_position_mismatch"
        );
    }

    /// Severity / source axes are independent — pin the cross-product
    /// for the four production variants so a future variant addition
    /// has to think about both axes deliberately.
    #[test]
    fn severity_source_cross_product_is_pinned() {
        let pua = Diagnostic::source_contains_pua(Span::new(0, 3), '\u{E001}');
        assert_eq!(pua.severity(), Severity::Warning);
        assert_eq!(pua.source(), DiagnosticSource::Source);

        let unclosed = Diagnostic::unclosed_bracket(Span::new(0, 3), PairKind::Bracket);
        assert_eq!(unclosed.severity(), Severity::Error);
        assert_eq!(unclosed.source(), DiagnosticSource::Source);

        let unmatched = Diagnostic::unmatched_close(Span::new(0, 3), PairKind::Bracket);
        assert_eq!(unmatched.severity(), Severity::Error);
        assert_eq!(unmatched.source(), DiagnosticSource::Source);

        let internal = Diagnostic::internal(Span::new(0, 3), InternalCheckCode::RegistryOutOfOrder);
        assert_eq!(internal.severity(), Severity::Error);
        assert_eq!(internal.source(), DiagnosticSource::Internal);
    }
}
