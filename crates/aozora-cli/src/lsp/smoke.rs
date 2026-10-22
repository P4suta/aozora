use tower_lsp::lsp_types::{DiagnosticSeverity, HoverContents, Position};

use super::diagnostics::diagnostics_for_source;
use super::formatting::format_edits;
use super::hover::hover_at;
use crate::i18n::LanguageIdentifier;

fn en() -> LanguageIdentifier {
    "en".parse().expect("en parses")
}

#[test]
fn plain_text_yields_no_diagnostics_and_no_edits() {
    let src = "hello world";
    assert!(diagnostics_for_source(src, &en()).is_empty());
    assert!(format_edits(src).is_empty());
}

#[test]
fn pua_collision_produces_warning_diagnostic() {
    let src = "oops\u{E001}here";
    let diags = diagnostics_for_source(src, &en());
    assert!(
        diags
            .iter()
            .any(|d| d.severity == Some(DiagnosticSeverity::WARNING)),
        "expected at least one warning diagnostic, got {diags:?}",
    );
}

#[test]
fn redundant_bar_ruby_reformats_to_bare() {
    let src = "｜日本《にほん》";
    let edits = format_edits(src);
    assert_eq!(edits.len(), 1, "a redundant ｜ should produce one edit");
    assert!(
        edits[0].new_text.starts_with("日本《にほん》") && !edits[0].new_text.contains('｜'),
        "reformats to bare ruby, got {:?}",
        edits[0].new_text,
    );
}

#[test]
fn canonical_ruby_reformats_to_itself() {
    let src = "日本《にほん》";
    assert!(format_edits(src).is_empty());
}

#[test]
fn hover_on_known_gaiji_mentions_resolved_character() {
    let src = "語※［＃「木＋吶のつくり」、第3水準1-85-54］で";
    let pos = Position::new(0, 3);
    let document = aozora::parse(src).expect("test source is within parser limit");
    let hover = hover_at(&document.snapshot(), pos, &en()).expect("hover must fire");
    let md = match hover.contents {
        HoverContents::Markup(m) => m.value,
        _ => panic!("expected Markdown hover"),
    };
    assert!(md.contains("枘") || md.contains("6798"));
}

#[test]
fn hover_outside_any_gaiji_returns_none() {
    let document = aozora::parse("ただの文").expect("test source is within parser limit");
    assert!(hover_at(&document.snapshot(), Position::new(0, 1), &en()).is_none());
}
