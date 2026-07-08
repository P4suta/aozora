//! aozora lexer diagnostic → LSP `Diagnostic` adapter.
//!
//! The diagnostic *catalogue* — codes, severities, the verbose Japanese
//! prose ([`aozora::Diagnostic::detail_body`]), the one-line titles and
//! documentation URLs ([`aozora::Diagnostic::explain`]) — is single-sourced
//! in `aozora-spec` (surfaced through the `aozora` facade), the same
//! authority `aozora check` and `aozora explain` render from. This module
//! is a thin adapter: it maps each [`aozora::Diagnostic`] onto a `tower_lsp`
//! [`Diagnostic`], converting byte spans into line/UTF-16 coordinates via a
//! [`DocLineView`] and attaching a serialised quick-fix [`DiagnosticPayload`]
//! for the `code_action` handler.
//!
//! `DiagnosticPayload` / `SerializablePairKind` live here (not in the
//! catalogue crate) because they are an LSP concern: the `data` channel and
//! the `WorkspaceEdit` the quick-fix builds. `code_action` imports them from
//! `crate::diagnostics`.

use aozora::{Diagnostic as AozoraDiagnostic, Document, InternalCheckCode, PairKind, Severity};
use serde::{Deserialize, Serialize};
use tower_lsp::lsp_types::{
    CodeDescription, Diagnostic, DiagnosticSeverity, DiagnosticTag, NumberOrString, Range, Url,
};

use crate::doc_line_view::DocLineView;

/// Serialised payload attached to an LSP diagnostic's `data` field. Lets the
/// `code_action` quick-fix handler build an edit without re-parsing or
/// re-classifying the offending span.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum DiagnosticPayload {
    /// `UnclosedBracket` — the open delimiter is here; the missing close is
    /// `expected_close`.
    UnclosedBracket {
        /// Which delimiter pair was left open.
        pair_kind: SerializablePairKind,
        /// The close delimiter that would balance it.
        expected_close: String,
    },
    /// `UnmatchedClose` — the close delimiter is here without a matching open.
    UnmatchedClose {
        /// Which delimiter pair the stray close belongs to.
        pair_kind: SerializablePairKind,
    },
    /// `SourceContainsPua` — a private-use codepoint clashes with the lexer's
    /// sentinel reservations.
    SourceContainsPua {
        /// The offending Unicode scalar value.
        codepoint: u32,
    },
    /// `ResidualAnnotationMarker` — a `［＃...］` pair survived classification
    /// (likely a typo or unsupported keyword); no automatic fix.
    ResidualAnnotationMarker,
    /// `NonCanonicalDirective` — the flagged `［＃…］` body is a verified
    /// near-miss; `canonical` is the catalogue spelling the quick-fix
    /// substitutes (bracket span and all).
    NonCanonicalDirective {
        /// The canonical directive body (without the `［＃` / `］` delimiters).
        canonical: String,
    },
}

/// Serde-stable stringification of [`PairKind`] for the `data` channel.
///
/// The delimiter glyphs themselves come from [`PairKind::open_str`] /
/// [`PairKind::close_str`] (the single authority); this enum only pins the
/// kebab-case wire tags the quick-fix payload round-trips through.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SerializablePairKind {
    /// `［` … `]`
    Bracket,
    /// `《` … `》`
    Ruby,
    /// `≪` … `≫`
    AngleQuote,
    /// `〔` … `〕`
    Tortoise,
    /// `「` … `」`
    Quote,
}

impl From<PairKind> for SerializablePairKind {
    fn from(k: PairKind) -> Self {
        // `PairKind` is `#[non_exhaustive]`; merging `Bracket` with the
        // wildcard keeps the fallback explicit without a duplicate-body arm.
        match k {
            PairKind::Ruby => Self::Ruby,
            PairKind::AngleQuote => Self::AngleQuote,
            PairKind::Tortoise => Self::Tortoise,
            PairKind::Quote => Self::Quote,
            PairKind::Bracket | _ => Self::Bracket,
        }
    }
}

impl SerializablePairKind {
    /// The `aozora-spec` [`PairKind`] this tag denotes.
    const fn pair_kind(self) -> PairKind {
        match self {
            Self::Bracket => PairKind::Bracket,
            Self::Ruby => PairKind::Ruby,
            Self::AngleQuote => PairKind::AngleQuote,
            Self::Tortoise => PairKind::Tortoise,
            Self::Quote => PairKind::Quote,
        }
    }

    /// Open delimiter literal — delegates to the [`PairKind`] authority.
    pub(crate) const fn open_str(self) -> &'static str {
        self.pair_kind().open_str()
    }

    /// Close delimiter literal — delegates to the [`PairKind`] authority.
    pub(crate) const fn close_str(self) -> &'static str {
        self.pair_kind().close_str()
    }
}

/// Parse `source` and return its diagnostics in LSP shape.
#[must_use]
pub fn diagnostics_for_source(source: &str) -> Vec<Diagnostic> {
    let document = Document::new(source);
    let tree = document.parse();
    let view = DocLineView::from_source(source);
    diagnostics_from_aozora(&view, tree.diagnostics())
}

/// Map a slice of pre-computed `aozora` [`AozoraDiagnostic`]s to LSP diagnostics.
///
/// The LSP backend's `publishDiagnostics` path uses this with the diagnostics
/// already held in the parse cache, skipping a re-parse. `view` maps each
/// diagnostic's byte span onto an LSP position — rope-backed on the publish
/// hot path (no per-keystroke line-table rebuild) or `&str`-backed elsewhere.
#[must_use]
pub fn diagnostics_from_aozora(
    view: &DocLineView<'_>,
    diagnostics: &[AozoraDiagnostic],
) -> Vec<Diagnostic> {
    diagnostics.iter().map(|d| to_lsp(view, d)).collect()
}

/// Map `aozora`'s [`Severity`] onto the LSP severity enum.
fn to_lsp_severity(severity: Severity) -> DiagnosticSeverity {
    match severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        // `Severity` is `#[non_exhaustive]`; `Note` and any future variant
        // surface as informational rather than failing the build here.
        Severity::Note | _ => DiagnosticSeverity::INFORMATION,
    }
}

/// Build the serialised quick-fix payload for a diagnostic, if it has a fix.
fn quick_fix_payload(d: &AozoraDiagnostic) -> Option<DiagnosticPayload> {
    match d {
        AozoraDiagnostic::SourceContainsPua { codepoint, .. } => {
            Some(DiagnosticPayload::SourceContainsPua {
                codepoint: *codepoint as u32,
            })
        }
        AozoraDiagnostic::UnclosedBracket { kind, .. } => {
            let pair_kind = SerializablePairKind::from(*kind);
            Some(DiagnosticPayload::UnclosedBracket {
                pair_kind,
                expected_close: pair_kind.close_str().to_owned(),
            })
        }
        AozoraDiagnostic::UnmatchedClose { kind, .. } => Some(DiagnosticPayload::UnmatchedClose {
            pair_kind: SerializablePairKind::from(*kind),
        }),
        AozoraDiagnostic::NonCanonicalDirective { canonical, .. } => {
            Some(DiagnosticPayload::NonCanonicalDirective {
                canonical: canonical.to_string(),
            })
        }
        AozoraDiagnostic::Internal {
            check: InternalCheckCode::ResidualAnnotationMarker,
            ..
        } => Some(DiagnosticPayload::ResidualAnnotationMarker),
        // Every other variant (including the other three internal checks) has
        // no automatic fix; `#[non_exhaustive]` requires the wildcard.
        _ => None,
    }
}

fn to_lsp(view: &DocLineView<'_>, d: &AozoraDiagnostic) -> Diagnostic {
    let span = d.span();
    let start = view.position(span.start as usize);
    let end = view.position(span.end as usize);
    // Title + URL come from the catalogue keyed by the code; the body is
    // rendered from this live diagnostic so its instance specifics (the
    // offending delimiter, codepoint, canonical spelling) are exact.
    let info = AozoraDiagnostic::explain(d.code())
        .expect("every emitted diagnostic code is catalogued in aozora-spec");
    let message = format!("{}\n\n{}", info.title, d.detail_body());
    let code_description = info
        .url
        .as_deref()
        .and_then(|u| Url::parse(u).ok())
        .map(|href| CodeDescription { href });
    Diagnostic {
        range: Range::new(start, end),
        severity: Some(to_lsp_severity(d.severity())),
        code: Some(NumberOrString::String(d.code().to_owned())),
        code_description,
        source: Some("aozora-lsp".to_owned()),
        message,
        tags: d.is_unnecessary().then(|| vec![DiagnosticTag::UNNECESSARY]),
        data: quick_fix_payload(d)
            .map(|p| serde_json::to_value(p).unwrap_or(serde_json::Value::Null)),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code_of(d: &Diagnostic) -> Option<&str> {
        match &d.code {
            Some(NumberOrString::String(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    #[test]
    fn plain_text_has_no_diagnostics() {
        assert!(diagnostics_for_source("hello world").is_empty());
    }

    #[test]
    fn canonical_ruby_has_no_diagnostics() {
        assert!(diagnostics_for_source("｜日本《にほん》").is_empty());
    }

    #[test]
    fn source_contains_pua_message_explains_what_to_do() {
        let src = "abc\u{E001}def";
        let diags = diagnostics_for_source(src);
        let pua = diags
            .iter()
            .find(|d| code_of(d) == Some("aozora::lex::source_contains_pua"))
            .expect("PUA warning expected");
        assert!(pua.message.contains("削除"), "msg: {}", pua.message);
        assert_eq!(pua.severity, Some(DiagnosticSeverity::WARNING));
        assert!(
            pua.tags
                .as_ref()
                .is_some_and(|t| t.contains(&DiagnosticTag::UNNECESSARY))
        );
        assert!(pua.data.is_some(), "data payload should be attached");
        assert!(
            pua.code_description.is_some(),
            "code_description (docs URL) should be attached"
        );
    }

    #[test]
    fn unclosed_bracket_message_carries_example_and_close_char() {
        // `［＃改ページ` (no closing ］) — must surface as UnclosedBracket.
        let src = "本文［＃改ページ";
        let diags = diagnostics_for_source(src);
        let unclosed = diags
            .iter()
            .find(|d| code_of(d) == Some("aozora::lex::unclosed_bracket"))
            .expect("UnclosedBracket expected on missing ］");
        assert!(unclosed.message.contains('］'), "{}", unclosed.message);
        assert!(
            unclosed.message.contains("例:"),
            "message must include a concrete example: {}",
            unclosed.message,
        );
        assert!(
            unclosed.data.is_some(),
            "data payload required for quick-fix"
        );
    }

    #[test]
    fn unmatched_close_message_lists_three_causes() {
        // `］` without a leading `［` — surfaces as UnmatchedClose.
        let src = "本文 ］";
        let diags = diagnostics_for_source(src);
        let unmatched = diags
            .iter()
            .find(|d| code_of(d) == Some("aozora::lex::unmatched_close"))
            .expect("UnmatchedClose expected on stray ］");
        assert!(unmatched.message.contains("削除"), "{}", unmatched.message);
        assert!(
            unmatched.message.contains("欠けている"),
            "{}",
            unmatched.message
        );
    }

    #[test]
    fn every_diagnostic_has_a_title_and_body() {
        // No generic fallthrough: every emitted diagnostic carries real
        // catalogue prose (a title line, a blank line, then the body).
        let src = "本文［＃改ページ";
        let diags = diagnostics_for_source(src);
        assert!(!diags.is_empty());
        for d in &diags {
            let (title, body) = d.message.split_once("\n\n").expect("title\\n\\nbody shape");
            assert!(!title.trim().is_empty(), "empty title: {}", d.message);
            assert!(!body.trim().is_empty(), "empty body: {}", d.message);
        }
    }

    #[test]
    fn diagnostic_carries_aozora_lsp_source_tag() {
        let src = "abc\u{E001}def";
        let diags = diagnostics_for_source(src);
        assert!(
            diags
                .iter()
                .all(|d| d.source.as_deref() == Some("aozora-lsp")),
            "every diagnostic must be tagged aozora-lsp: {diags:?}",
        );
    }

    #[test]
    fn severity_maps_to_lsp_for_every_variant() {
        assert_eq!(to_lsp_severity(Severity::Error), DiagnosticSeverity::ERROR);
        assert_eq!(
            to_lsp_severity(Severity::Warning),
            DiagnosticSeverity::WARNING
        );
        assert_eq!(
            to_lsp_severity(Severity::Note),
            DiagnosticSeverity::INFORMATION
        );
    }

    #[test]
    fn codes_use_the_canonical_dotted_form() {
        // Editor `code` strings match the CLI / book / `aozora explain` set.
        let diags = diagnostics_for_source("本文 ］");
        assert!(
            diags.iter().all(|d| {
                code_of(d).is_some_and(|c| {
                    c.starts_with("aozora::lex::") || c.starts_with("aozora::lint::")
                })
            }),
            "codes must be dotted aozora::lex::* / aozora::lint::*: {diags:?}",
        );
    }
}
