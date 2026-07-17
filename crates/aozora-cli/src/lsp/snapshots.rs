//! Snapshot tests that lock the *structured* output of the read-only LSP
//! providers on a curated corpus — the diagnostic catalogue, the semantic-
//! token delta stream, the document-symbol outline — plus the capability set
//! the server advertises. A change in any of them becomes a reviewable
//! `cargo insta` diff rather than a silent regression.
//!
//! For the providers we snapshot **projections** (codes/severities/ranges,
//! token tuples, outline names) rather than the full LSP structs: the
//! projection is the load-bearing contract, and it stays readable and stable
//! while the verbose Japanese diagnostic prose (pinned separately by unit
//! tests) can evolve without churning every snapshot. The capability set is
//! the one value serialized whole — `lsp-types` omits `None` options, so its
//! JSON already *is* the projection: every key present is a capability
//! offered.

use tower_lsp::lsp_types::{DiagnosticSeverity, DocumentSymbol, NumberOrString};

use super::capabilities::{server_capabilities, server_info};
use super::diagnostics::diagnostics_for_source;
use super::document_symbol::document_symbols;
use super::semantic_tokens::semantic_tokens_full;
use super::state::OpenDocument;
use crate::i18n::LanguageIdentifier;

/// The projections snapshotted here (codes/ranges, token tuples, outline
/// names) are locale-independent, but the providers are now locale-
/// parameterised — pin English so the snapshot stays deterministic.
fn en() -> LanguageIdentifier {
    "en".parse().expect("en parses")
}

/// Headings (大見出し / 中見出し) plus ruby — drives the symbol outline.
const HEADINGS: &str =
    "［＃大見出し］第一章\n\n本文です。\n\n［＃中見出し］第一節\n\n｜青空《あおぞら》の続き。";
/// An unclosed bracket and a PUA sentinel — drives the diagnostic catalogue.
const DIAG_SOURCE: &str = "本文［＃改ページ\nまだ\u{E001}閉じない";
/// Explicit ruby + a gaiji reference — drives the semantic-token stream.
const RUBY_GAIJI: &str = "｜青空《あおぞら》と※［＃「弓＋鳥」、第3水準1-2-3］。";

fn severity_label(severity: Option<DiagnosticSeverity>) -> &'static str {
    match severity {
        Some(DiagnosticSeverity::ERROR) => "ERROR",
        Some(DiagnosticSeverity::WARNING) => "WARNING",
        Some(DiagnosticSeverity::INFORMATION) => "INFO",
        Some(DiagnosticSeverity::HINT) => "HINT",
        _ => "?",
    }
}

/// `code | severity | startLine:startChar..endLine:endChar` per diagnostic.
fn diagnostic_view(src: &str) -> Vec<String> {
    diagnostics_for_source(src, &en())
        .iter()
        .map(|d| {
            let code = match &d.code {
                Some(NumberOrString::String(s)) => s.clone(),
                Some(NumberOrString::Number(n)) => n.to_string(),
                None => "<none>".to_owned(),
            };
            format!(
                "{code} | {} | {}:{}..{}:{}",
                severity_label(d.severity),
                d.range.start.line,
                d.range.start.character,
                d.range.end.line,
                d.range.end.character,
            )
        })
        .collect()
}

/// `(delta_line, delta_start, length, token_type, modifiers)` per token.
fn semantic_token_view(src: &str) -> Vec<(u32, u32, u32, u32, u32)> {
    let doc = OpenDocument::new(src.to_owned());
    let snap = doc.snapshot();
    semantic_tokens_full(&snap.paragraphs)
        .data
        .iter()
        .map(|t| {
            (
                t.delta_line,
                t.delta_start,
                t.length,
                t.token_type,
                t.token_modifiers_bitset,
            )
        })
        .collect()
}

/// Indented `name [kind]` outline, depth-first.
fn symbol_outline(src: &str) -> Vec<String> {
    fn walk(symbols: &[DocumentSymbol], depth: usize, out: &mut Vec<String>) {
        for symbol in symbols {
            out.push(format!(
                "{}{} [{:?}]",
                "  ".repeat(depth),
                symbol.name,
                symbol.kind
            ));
            if let Some(children) = &symbol.children {
                walk(children, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(&document_symbols(src, &en()), 0, &mut out);
    out
}

#[test]
fn snapshot_diagnostic_catalogue() {
    insta::assert_debug_snapshot!(diagnostic_view(DIAG_SOURCE));
}

#[test]
fn snapshot_semantic_token_stream() {
    insta::assert_debug_snapshot!(semantic_token_view(RUBY_GAIJI));
}

#[test]
fn snapshot_document_symbol_outline() {
    insta::assert_debug_snapshot!(symbol_outline(HEADINGS));
}

#[test]
fn snapshot_server_capabilities() {
    insta::assert_json_snapshot!(server_capabilities());
}

/// `server_info` is deliberately *not* snapshotted — it carries the crate
/// version, so a snapshot would churn on every release. Pin the two things
/// that are actually contracts instead: the name clients display, and that the
/// version is the crate's own rather than a literal that can drift behind it.
#[test]
fn server_info_names_the_crate_and_its_version() {
    let info = server_info();
    assert_eq!(info.name, "aozora-lsp");
    assert_eq!(info.version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
}
