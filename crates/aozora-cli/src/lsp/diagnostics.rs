#![expect(
    clippy::expect_used,
    reason = "diagnostic codes are normalized to a non-empty final segment"
)]

use aozora::InternalCheckCode;
use aozora::{Diagnostic as AozoraDiagnostic, PairKind, Severity};

use crate::i18n::{self as i18n, FluentArgs, LanguageIdentifier};
use serde::{Deserialize, Serialize};
use tower_lsp::lsp_types::{
    CodeDescription, Diagnostic, DiagnosticSeverity, DiagnosticTag, NumberOrString, Range, Url,
};

use crate::lsp::doc_line_view::DocLineView;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(crate) enum DiagnosticPayload {
    UnclosedBracket {
        pair_kind: SerializablePairKind,
        expected_close: String,
    },
    UnmatchedClose {
        pair_kind: SerializablePairKind,
    },
    SourceContainsPua {
        codepoint: u32,
    },
    ResidualAnnotationMarker,
    NonCanonicalDirective {
        canonical: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SerializablePairKind {
    Bracket,
    Ruby,
    AngleQuote,
    Tortoise,
    Quote,
}

impl TryFrom<PairKind> for SerializablePairKind {
    type Error = ();

    fn try_from(k: PairKind) -> Result<Self, Self::Error> {
        match k {
            PairKind::Bracket => Ok(Self::Bracket),
            PairKind::Ruby => Ok(Self::Ruby),
            PairKind::AngleQuote => Ok(Self::AngleQuote),
            PairKind::Tortoise => Ok(Self::Tortoise),
            PairKind::Quote => Ok(Self::Quote),
            _ => Err(()),
        }
    }
}

impl SerializablePairKind {
    const fn pair_kind(self) -> PairKind {
        match self {
            Self::Bracket => PairKind::Bracket,
            Self::Ruby => PairKind::Ruby,
            Self::AngleQuote => PairKind::AngleQuote,
            Self::Tortoise => PairKind::Tortoise,
            Self::Quote => PairKind::Quote,
        }
    }

    pub(crate) const fn open_str(self) -> &'static str {
        self.pair_kind().open_str()
    }

    pub(crate) const fn close_str(self) -> &'static str {
        self.pair_kind().close_str()
    }
}

#[cfg(test)]
#[must_use]
pub(crate) fn diagnostics_for_source(source: &str, lang: &LanguageIdentifier) -> Vec<Diagnostic> {
    let document = aozora::parse(source).expect("source fits parser span limit");
    let tree = document.snapshot();
    let view = DocLineView::from_source(source);
    diagnostics_from_aozora(&view, tree.diagnostics(), lang)
}

#[must_use]
pub(super) fn diagnostics_from_aozora(
    view: &DocLineView<'_>,
    diagnostics: &[AozoraDiagnostic],
    lang: &LanguageIdentifier,
) -> Vec<Diagnostic> {
    diagnostics.iter().map(|d| to_lsp(view, d, lang)).collect()
}

fn to_lsp_severity(severity: Severity) -> DiagnosticSeverity {
    match severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        Severity::Note | _ => DiagnosticSeverity::INFORMATION,
    }
}

fn quick_fix_payload(d: &AozoraDiagnostic) -> Option<DiagnosticPayload> {
    match d {
        AozoraDiagnostic::SourceContainsPua { codepoint, .. } => {
            Some(DiagnosticPayload::SourceContainsPua {
                codepoint: *codepoint as u32,
            })
        }
        AozoraDiagnostic::UnclosedBracket { kind, .. } => {
            let pair_kind = SerializablePairKind::try_from(*kind).ok()?;
            Some(DiagnosticPayload::UnclosedBracket {
                pair_kind,
                expected_close: pair_kind.close_str().to_owned(),
            })
        }
        AozoraDiagnostic::UnmatchedClose { kind, .. } => Some(DiagnosticPayload::UnmatchedClose {
            pair_kind: SerializablePairKind::try_from(*kind).ok()?,
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
        _ => None,
    }
}

fn serialize_payload(payload: DiagnosticPayload) -> serde_json::Value {
    match serde_json::to_value(payload) {
        Ok(value) => value,
        Err(error) => unreachable!("LSP diagnostic payload serialization failed: {error}"),
    }
}

fn to_lsp(view: &DocLineView<'_>, d: &AozoraDiagnostic, lang: &LanguageIdentifier) -> Diagnostic {
    let span = d.span();
    let start = view.position(span.start as usize);
    let end = view.position(span.end as usize);
    let info = AozoraDiagnostic::explain(d.code())
        .expect("every emitted diagnostic code is catalogued in aozora-spec");
    let title = i18n::diag_title(lang, d.code());
    let mut body_args = FluentArgs::new();
    for (name, value) in d.body_args() {
        body_args.set(name, value.into_owned());
    }
    let body = i18n::diag_body(lang, d.code(), &body_args);
    let message = format!("{title}\n\n{body}");
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
        data: quick_fix_payload(d).map(serialize_payload),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use aozora::Span;
    use tower_lsp::lsp_types::Position;

    use super::*;

    fn en() -> LanguageIdentifier {
        "en".parse().expect("en parses")
    }

    fn diagnostics_for_source(source: &str) -> Vec<Diagnostic> {
        super::diagnostics_for_source(source, &en())
    }

    fn code_of(d: &Diagnostic) -> Option<&str> {
        match &d.code {
            Some(NumberOrString::String(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    #[test]
    fn plain_text_has_no_diagnostics() {
        assert_eq!(
            diagnostics_for_source("hello world"),
            Vec::<Diagnostic>::new()
        );
    }

    #[test]
    fn canonical_ruby_has_no_diagnostics() {
        assert_eq!(
            diagnostics_for_source("｜日本《にほん》"),
            Vec::<Diagnostic>::new()
        );
    }

    #[test]
    fn source_contains_pua_message_explains_what_to_do() {
        let src = "abc\u{E001}def";
        let diags = diagnostics_for_source(src);
        let pua = diags
            .iter()
            .find(|d| code_of(d) == Some("aozora::lex::source_contains_pua"))
            .expect("PUA warning expected");
        assert!(pua.message.contains("delete"), "msg: {}", pua.message);
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
        let src = "本文［＃改ページ";
        let diags = diagnostics_for_source(src);
        let unclosed = diags
            .iter()
            .find(|d| code_of(d) == Some("aozora::lex::unclosed_bracket"))
            .expect("UnclosedBracket expected on missing ］");
        assert!(unclosed.message.contains('］'), "{}", unclosed.message);
        assert!(
            unclosed.message.contains("Example:"),
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
        let src = "本文 ］";
        let diags = diagnostics_for_source(src);
        let unmatched = diags
            .iter()
            .find(|d| code_of(d) == Some("aozora::lex::unmatched_close"))
            .expect("UnmatchedClose expected on stray ］");
        assert!(
            unmatched.message.contains("delete"),
            "{}",
            unmatched.message
        );
        assert!(
            unmatched.message.contains("missing"),
            "{}",
            unmatched.message
        );
    }

    #[test]
    fn every_diagnostic_has_a_title_and_body() {
        let src = "本文［＃改ページ";
        let diags = diagnostics_for_source(src);
        assert_ne!(diags, Vec::<Diagnostic>::new());
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
    fn serializable_pair_kind_open_and_close_str_match_authority() {
        let cases = [
            (SerializablePairKind::Bracket, "［", "］"),
            (SerializablePairKind::Ruby, "《", "》"),
            (SerializablePairKind::AngleQuote, "≪", "≫"),
            (SerializablePairKind::Tortoise, "〔", "〕"),
            (SerializablePairKind::Quote, "「", "」"),
        ];
        for (kind, open, close) in cases {
            assert_eq!(kind.open_str(), open, "open_str for {kind:?}");
            assert_eq!(kind.close_str(), close, "close_str for {kind:?}");
        }
    }

    #[test]
    fn serializable_pair_kind_covers_every_core_pair_kind() {
        for &kind in PairKind::ALL {
            let serializable = SerializablePairKind::try_from(kind)
                .expect("every core pair kind has an LSP payload tag");
            assert_eq!(serializable.open_str(), kind.open_str());
            assert_eq!(serializable.close_str(), kind.close_str());
        }
    }

    #[test]
    fn quick_fix_payload_maps_unmatched_close_to_its_pair_kind() {
        let d = AozoraDiagnostic::unmatched_close(Span::new(3, 6), PairKind::Ruby);
        let payload = quick_fix_payload(&d);
        assert!(
            matches!(
                payload,
                Some(DiagnosticPayload::UnmatchedClose {
                    pair_kind: SerializablePairKind::Ruby
                })
            ),
            "UnmatchedClose must yield a quick-fix payload carrying its pair kind, got {payload:?}",
        );
    }

    #[test]
    fn quick_fix_payload_carries_canonical_for_non_canonical_directive() {
        let d = AozoraDiagnostic::non_canonical_directive(Span::new(0, 9), "中央揃え");
        let payload = quick_fix_payload(&d);
        let Some(DiagnosticPayload::NonCanonicalDirective { canonical }) = payload else {
            panic!("NonCanonicalDirective must yield a NonCanonicalDirective quick-fix payload");
        };
        assert_eq!(canonical, "中央揃え");
    }

    #[test]
    fn quick_fix_payload_maps_residual_annotation_marker() {
        let d = AozoraDiagnostic::internal(
            Span::new(0, 1),
            InternalCheckCode::ResidualAnnotationMarker,
        );
        let payload = quick_fix_payload(&d);
        assert!(
            matches!(payload, Some(DiagnosticPayload::ResidualAnnotationMarker)),
            "ResidualAnnotationMarker internal check must yield its payload, got {payload:?}",
        );
    }

    #[test]
    fn serialized_quick_fix_payload_pins_the_lsp_wire_shape() {
        let value = serialize_payload(DiagnosticPayload::UnclosedBracket {
            pair_kind: SerializablePairKind::Bracket,
            expected_close: "］".to_owned(),
        });

        assert_eq!(
            value,
            serde_json::json!({
                "kind": "unclosed-bracket",
                "pair_kind": "bracket",
                "expected_close": "］",
            }),
        );
    }

    #[test]
    fn to_lsp_range_reflects_diagnostic_span_not_default() {
        let source = "𠮷文］";
        let view = DocLineView::from_source(source);
        let d = AozoraDiagnostic::unmatched_close(Span::new(7, 10), PairKind::Bracket);
        let lang = i18n::resolve(None, None, None, None);
        let lsp = to_lsp(&view, &d, &lang);
        assert_eq!(
            lsp.range,
            Range::new(Position::new(0, 3), Position::new(0, 4)),
            "range must reflect the diagnostic span, not Range::default()",
        );
    }

    #[test]
    fn diagnostic_prose_follows_the_requested_lang() {
        let src = "本文［＃改ページ";
        let ja: LanguageIdentifier = "ja".parse().expect("ja parses");
        let msg = |lang: &LanguageIdentifier| {
            super::diagnostics_for_source(src, lang)
                .into_iter()
                .find(|d| code_of(d) == Some("aozora::lex::unclosed_bracket"))
                .map(|d| d.message)
                .expect("unclosed_bracket diagnostic present")
        };
        let en_msg = msg(&en());
        let ja_msg = msg(&ja);
        assert!(en_msg.contains("Unclosed"), "en headline: {en_msg}");
        assert!(ja_msg.contains("閉じ"), "ja headline: {ja_msg}");
        assert_ne!(en_msg, ja_msg, "the requested lang must change the prose");
    }

    #[test]
    fn codes_use_the_canonical_dotted_form() {
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
