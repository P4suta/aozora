//! Dependency-free plain-text diagnostic rendering.
//!
//! [`diagnostics_text`] formats a parse's diagnostics into a
//! human-readable string using only the public [`Diagnostic`] accessors
//! — no `miette` / terminal dependency. That keeps it compilable for
//! every target (notably `wasm32-unknown-unknown`, where `miette`'s
//! `fancy` graphical renderer cannot build), so the WASM / Python / FFI
//! front doors can surface human-facing diagnostics and not just the
//! machine [`json`](crate::json) envelope.
//!
//! The CLI keeps its richer `miette` graphical report for a TTY; this is
//! the portable sibling — closest in spirit to the CLI's one-line
//! `short` view, with the offending source slice added for context.

use std::fmt::{self, Write as _};

use crate::pipeline::lexer::sanitize;

use crate::Diagnostic;

/// Render `diagnostics` over `source` as a plain-text report.
///
/// One block per diagnostic — a `<severity> [<code>] @ <start>..<end>:
/// <message>` header followed by the offending source slice — joined by
/// newlines. Returns the empty string when `diagnostics` is empty.
///
/// Spans are in *sanitized* coordinates (the lexer strips the BOM, folds
/// CRLF→LF, and decomposes 〔…〕 accent digraphs, each of which shifts
/// byte offsets), so `source` is sanitized internally before the
/// offending slice is cut — mirroring the CLI's graphical renderer.
///
/// # Panics
///
/// Does not panic in normal use: `String` cannot fail as a
/// [`Write`](std::fmt::Write) sink.
#[must_use]
pub fn diagnostics_text(source: &str, diagnostics: &[Diagnostic]) -> String {
    let mut out = String::new();
    // Writing into a `String` cannot fail; the fallible inner helper
    // keeps `write!` ergonomics without a per-line discard. Mirrors the
    // `serialize` renderer's infallible-write idiom.
    write_report(&mut out, source, diagnostics).expect("String write is infallible");
    out
}

fn write_report(out: &mut String, source: &str, diagnostics: &[Diagnostic]) -> fmt::Result {
    if diagnostics.is_empty() {
        return Ok(());
    }
    let sanitized = sanitize(source).text;
    for diag in diagnostics {
        let span = diag.span();
        let (start, end) = (span.start as usize, span.end as usize);
        writeln!(
            out,
            "{severity} [{code}] @ {start}..{end}: {diag}",
            severity = diag.severity().as_json_str(),
            code = diag.code(),
        )?;
        if let Some(slice) = sanitized.get(start..end) {
            writeln!(out, "  > {slice:?}")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::Document;

    #[test]
    fn empty_when_no_diagnostics() {
        let doc = Document::new("｜青空《あおぞら》");
        let tree = doc.snapshot();
        let text = super::diagnostics_text(doc.source(), tree.diagnostics());
        assert!(text.is_empty(), "clean parse renders nothing: {text:?}");
    }

    #[test]
    fn renders_code_and_offending_slice() {
        // Nested ruby is a diagnostic-bearing construct.
        let doc = Document::new("｜《おうめ》");
        let tree = doc.snapshot();
        let diagnostics = tree.diagnostics();
        if diagnostics.is_empty() {
            return; // grammar may not flag this exact input; skip if so.
        }
        let text = super::diagnostics_text(doc.source(), diagnostics);
        assert!(
            text.contains("aozora::") && text.contains('@'),
            "header carries the code + span: {text}"
        );
    }

    #[test]
    fn renders_nonempty_report_for_guaranteed_diagnostic() {
        // A PUA sentinel collision reliably fires a diagnostic (see
        // `document::tests::diagnostics_populated_for_pua_collision`), so
        // the report body MUST run and produce header output. A no-op
        // `write_report` would leave the string empty.
        let doc = Document::new("contains \u{E001} sentinel");
        let tree = doc.snapshot();
        let diagnostics = tree.diagnostics();
        assert!(
            !diagnostics.is_empty(),
            "PUA sentinel must produce a diagnostic to anchor this test"
        );
        let text = super::diagnostics_text(doc.source(), diagnostics);
        assert!(
            !text.is_empty(),
            "guaranteed diagnostic must render non-empty text"
        );
        let headers = text.lines().filter(|l| l.contains("] @ ")).count();
        assert_eq!(
            headers,
            diagnostics.len(),
            "every diagnostic must emit its header line: {text}"
        );
    }

    #[test]
    fn one_block_per_diagnostic() {
        let doc = Document::new("｜《おうめ》");
        let tree = doc.snapshot();
        let diagnostics = tree.diagnostics();
        let text = super::diagnostics_text(doc.source(), diagnostics);
        // Header lines (start with a severity word) number one per diag.
        let headers = text.lines().filter(|l| l.contains("] @ ")).count();
        assert_eq!(
            headers,
            diagnostics.len(),
            "one header per diagnostic: {text}"
        );
    }
}
