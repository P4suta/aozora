#![expect(
    clippy::expect_used,
    reason = "diagnostic codes are normalized to a non-empty final segment"
)]

//! aozora lexer diagnostic → LSP `Diagnostic` adapter.
//!
//! The machine *catalogue* — codes, severities, documentation URLs — is
//! single-sourced in the `aozora` spec layer (surfaced through the crate
//! facade), the same authority `aozora check` and `aozora explain` render
//! from. The human-facing prose — the one-line title and the verbose body —
//! comes from the shared Fluent localization catalog keyed by diagnostic code, the same
//! catalog the CLI's `explain` and human report use. This module is a thin
//! adapter: it maps each [`aozora::Diagnostic`] onto a `tower_lsp`
//! [`Diagnostic`], converting byte spans into line/UTF-16 coordinates via a
//! [`DocLineView`] and attaching a serialised quick-fix [`DiagnosticPayload`]
//! for the `code_action` handler.
//!
//! The message language is the server's UI language (`super::server_locale::ui_lang`,
//! resolved once at startup from `AOZORA_LANG > LANG > en`), threaded in as
//! `lang` by the backend so a `ja`/`zh` editor gets localized diagnostic
//! prose. The machine axis — code, severity, span, documentation URL — is
//! unchanged by the locale.
//!
//! `DiagnosticPayload` / `SerializablePairKind` live here (not in the
//! catalogue crate) because they are an LSP concern: the `data` channel and
//! the `WorkspaceEdit` the quick-fix builds. `code_action` imports them from
//! `crate::lsp::diagnostics`.

use aozora::InternalCheckCode;
// `Document` is reached only by the `#[cfg(test)]` `diagnostics_for_source`.
use aozora::{Diagnostic as AozoraDiagnostic, PairKind, Severity};

use crate::i18n::{self as i18n, FluentArgs, LanguageIdentifier};
use serde::{Deserialize, Serialize};
use tower_lsp::lsp_types::{
    CodeDescription, Diagnostic, DiagnosticSeverity, DiagnosticTag, NumberOrString, Range, Url,
};

use crate::lsp::doc_line_view::DocLineView;

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
    /// The [`PairKind`] this tag denotes.
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

/// Parse `source` and return its diagnostics in LSP shape, with the human
/// prose rendered in `lang`. A `&str`-source convenience used by the in-module
/// tests; the server handlers go through [`diagnostics_from_aozora`] directly.
#[cfg(test)]
#[must_use]
pub(crate) fn diagnostics_for_source(source: &str, lang: &LanguageIdentifier) -> Vec<Diagnostic> {
    let document = aozora::parse(source).expect("source fits parser span limit");
    let tree = document.snapshot();
    let view = DocLineView::from_source(source);
    diagnostics_from_aozora(&view, tree.diagnostics(), lang)
}

/// Map a slice of pre-computed `aozora` [`AozoraDiagnostic`]s to LSP diagnostics.
///
/// The LSP backend's `publishDiagnostics` path uses this with the diagnostics
/// already held in the parse cache, skipping a re-parse. `view` maps each
/// diagnostic's byte span onto an LSP position — rope-backed on the publish
/// hot path (no per-keystroke line-table rebuild) or `&str`-backed elsewhere.
#[must_use]
pub(super) fn diagnostics_from_aozora(
    view: &DocLineView<'_>,
    diagnostics: &[AozoraDiagnostic],
    lang: &LanguageIdentifier,
) -> Vec<Diagnostic> {
    // `lang` is the server's UI language, resolved once at startup and threaded
    // in by the backend — the whole batch renders in one locale.
    diagnostics.iter().map(|d| to_lsp(view, d, lang)).collect()
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
        // Every other variant (including the other three internal checks) has
        // no automatic fix; `#[non_exhaustive]` requires the wildcard.
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
    // Title + body come from the shared i18n catalog keyed by the code; the
    // body's instance specifics (the offending delimiter, codepoint, canonical
    // spelling) are interpolated from this live diagnostic's `body_args`. The
    // documentation URL is the machine axis, read from the catalogue crate.
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

    /// Shim mirroring the pre-i18n `diagnostics_for_source(source)` arity,
    /// pinned to English so the prose assertions below stay locale-stable
    /// (they already asserted the canonical English wording).
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
        // `［＃改ページ` (no closing ］) — must surface as UnclosedBracket.
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
        // `］` without a leading `［` — surfaces as UnmatchedClose.
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
    fn serializable_pair_kind_open_and_close_str_match_authority() {
        // Each wire tag stringifies to the exact `PairKind` glyph via the
        // spec authority; a stubbed body (`""` / `"xyzzy"`) is caught per
        // variant for both `open_str` and `close_str`.
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
        // Deleting the `UnmatchedClose` arm drops the payload to `None`
        // via the `_` fallthrough; pin both presence and the carried kind.
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
        // Deleting the `NonCanonicalDirective` arm drops the payload to
        // `None`; the canonical spelling must round-trip into the payload.
        let d = AozoraDiagnostic::non_canonical_directive(Span::new(0, 9), "中央揃え");
        let payload = quick_fix_payload(&d);
        let Some(DiagnosticPayload::NonCanonicalDirective { canonical }) = payload else {
            panic!("NonCanonicalDirective must yield a NonCanonicalDirective quick-fix payload");
        };
        assert_eq!(canonical, "中央揃え");
    }

    #[test]
    fn quick_fix_payload_maps_residual_annotation_marker() {
        // Deleting the `Internal { ResidualAnnotationMarker }` arm drops the
        // payload to `None`; pin that this internal check gets its payload.
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
    fn to_lsp_range_reflects_diagnostic_span_not_default() {
        // `𠮷` is astral (4 bytes, 2 UTF-16 units); `文` is BMP (3 bytes,
        // 1 unit). The stray close `］` occupies bytes 7..10, i.e. UTF-16
        // columns 3..4 — distinct from any byte-offset reading and from
        // `Range::default()` = (0,0)-(0,0), so dropping the `range` field
        // from the built `Diagnostic` is caught.
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
        // W7: the LSP threads the server lang into the shared catalog. The same
        // diagnostic renders an English headline by default and a Japanese one
        // under `ja`; the code / severity / range are unchanged by the locale.
        let src = "本文［＃改ページ"; // unclosed bracket
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
