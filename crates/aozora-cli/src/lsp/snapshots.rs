use tower_lsp::lsp_types::{DiagnosticSeverity, DocumentSymbol, NumberOrString};

use super::capabilities::{server_capabilities, server_info};
use super::diagnostics::diagnostics_for_source;
use super::document_symbol::document_symbols;
use super::semantic_tokens::semantic_tokens_full;
use crate::i18n::LanguageIdentifier;

fn en() -> LanguageIdentifier {
    "en".parse().expect("en parses")
}

const HEADINGS: &str =
    "［＃大見出し］第一章\n\n本文です。\n\n［＃中見出し］第一節\n\n｜青空《あおぞら》の続き。";
const DIAG_SOURCE: &str = "本文［＃改ページ\nまだ\u{E001}閉じない";
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

fn semantic_token_view(src: &str) -> Vec<(u32, u32, u32, u32, u32)> {
    let document = aozora::parse(src).expect("snapshot source fits parser limits");
    semantic_tokens_full(&document.snapshot())
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

#[test]
fn server_info_names_the_crate_and_its_version() {
    let info = server_info();
    assert_eq!(info.name, "aozora-lsp");
    assert_eq!(info.version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
}
