//! aozora lexer diagnostic → LSP `Diagnostic` adapter.
//!
//! The diagnostic *catalogue* (codes, severities, the verbose Japanese
//! messages, quick-fix payloads, and the long-form `explain` prose) lives in
//! the renderer-agnostic [`aozora_diagnostics`] crate so the LSP and the CLI
//! share one source of truth. This module is a thin adapter: it calls
//! [`aozora_diagnostics::describe`] and maps the neutral [`Described`] record
//! onto a `tower_lsp` [`Diagnostic`], converting byte spans into line/UTF-16
//! coordinates via [`LineIndex`].
//!
//! `DiagnosticPayload` / `SerializablePairKind` are re-exported from
//! [`aozora_diagnostics`] so the `code_action` handler can keep importing them
//! from `crate::diagnostics`.

use aozora::{Diagnostic as AozoraDiagnostic, Document};
use aozora_diagnostics::{Described, Severity, describe};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, DiagnosticTag, NumberOrString, Range};

pub(crate) use aozora_diagnostics::{DiagnosticPayload, SerializablePairKind};

use crate::line_index::LineIndex;

/// Parse `source` and return its diagnostics in LSP shape.
#[must_use]
pub fn diagnostics_for_source(source: &str) -> Vec<Diagnostic> {
    let document = Document::new(source);
    let tree = document.parse();
    diagnostics_from_aozora(source, tree.diagnostics())
}

/// Map a slice of pre-computed `aozora` [`AozoraDiagnostic`]s to LSP diagnostics.
///
/// The LSP backend's `publishDiagnostics` path uses this with the diagnostics
/// already held in the parse cache, skipping a re-parse.
#[must_use]
pub fn diagnostics_from_aozora(source: &str, diagnostics: &[AozoraDiagnostic]) -> Vec<Diagnostic> {
    let line_index = LineIndex::new(source);
    diagnostics
        .iter()
        .map(|d| to_lsp(source, &line_index, d))
        .collect()
}

/// Map `aozora_diagnostics::Severity` onto the LSP severity enum.
fn to_lsp_severity(severity: Severity) -> DiagnosticSeverity {
    match severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
    }
}

fn to_lsp(source: &str, line_index: &LineIndex, d: &AozoraDiagnostic) -> Diagnostic {
    let described: Described = describe(d);
    let start = line_index.position(source, described.span.start as usize);
    let end = line_index.position(source, described.span.end as usize);
    Diagnostic {
        range: Range::new(start, end),
        severity: Some(to_lsp_severity(described.severity)),
        code: Some(NumberOrString::String(described.code.to_owned())),
        source: Some("aozora-lsp".to_owned()),
        message: described.message,
        tags: described
            .unnecessary
            .then(|| vec![DiagnosticTag::UNNECESSARY]),
        data: described
            .payload
            .map(|p| serde_json::to_value(p).unwrap_or(serde_json::Value::Null)),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            .find(|d| {
                matches!(
                    &d.code,
                    Some(NumberOrString::String(s)) if s == "aozora::source-contains-pua"
                )
            })
            .expect("PUA warning expected");
        assert!(pua.message.contains("削除"), "msg: {}", pua.message);
        assert_eq!(pua.severity, Some(DiagnosticSeverity::WARNING));
        assert!(
            pua.tags
                .as_ref()
                .is_some_and(|t| t.contains(&DiagnosticTag::UNNECESSARY))
        );
        assert!(pua.data.is_some(), "data payload should be attached");
    }

    #[test]
    fn unclosed_bracket_message_carries_example_and_close_char() {
        // `［＃改ページ` (no closing ］) — must surface as UnclosedBracket.
        let src = "本文［＃改ページ";
        let diags = diagnostics_for_source(src);
        let unclosed = diags
            .iter()
            .find(|d| {
                matches!(
                    &d.code,
                    Some(NumberOrString::String(s)) if s == "aozora::unclosed-bracket"
                )
            })
            .expect("UnclosedBracket expected on missing ］");
        assert!(unclosed.message.contains("］"), "{}", unclosed.message);
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
            .find(|d| {
                matches!(
                    &d.code,
                    Some(NumberOrString::String(s)) if s == "aozora::unmatched-close"
                )
            })
            .expect("UnmatchedClose expected on stray ］");
        assert!(unmatched.message.contains("削除"), "{}", unmatched.message);
        assert!(
            unmatched.message.contains("欠けている"),
            "{}",
            unmatched.message
        );
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
    fn severity_maps_to_lsp_for_both_variants() {
        assert_eq!(to_lsp_severity(Severity::Error), DiagnosticSeverity::ERROR);
        assert_eq!(
            to_lsp_severity(Severity::Warning),
            DiagnosticSeverity::WARNING
        );
    }
}
