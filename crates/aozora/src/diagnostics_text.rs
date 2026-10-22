#![expect(
    clippy::expect_used,
    reason = "diagnostic rendering writes into an infallible String buffer"
)]

use std::fmt::{self, Write as _};

use crate::Diagnostic;

/// Render diagnostics as plain text with source excerpts.
///
/// Out-of-range spans or spans inside a UTF-8 codepoint omit the excerpt.
///
/// # Panics
///
/// Panics only if writing to a `String` fails.
#[must_use]
pub fn diagnostics_text(source: &str, diagnostics: &[Diagnostic]) -> String {
    let mut out = String::new();
    write_report(&mut out, source, diagnostics).expect("String write is infallible");
    out
}

fn write_report(out: &mut String, source: &str, diagnostics: &[Diagnostic]) -> fmt::Result {
    if diagnostics.is_empty() {
        return Ok(());
    }
    for diag in diagnostics {
        let span = diag.span();
        let (start, end) = (span.start as usize, span.end as usize);
        writeln!(
            out,
            "{severity} [{code}] @ {start}..{end}: {diag}",
            severity = diag.severity().as_json_str(),
            code = diag.code(),
        )?;
        if let Some(slice) = source.get(start..end) {
            writeln!(out, "  > {slice:?}")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::Document;
    use crate::pipeline::lexer::sanitize;

    fn assert_source_coordinate_slice(source: &str, expected: &str) {
        let doc = Document::new(source);
        let tree = doc.snapshot();
        let diagnostics = tree.diagnostics();
        assert_eq!(
            diagnostics.len(),
            1,
            "fixture must yield exactly one diagnostic: {source:?}"
        );
        let span = diagnostics[0].span();
        let (start, end) = (span.start as usize, span.end as usize);
        assert_eq!(
            source.get(start..end),
            Some(expected),
            "source-coordinate span must bracket the expected source slice"
        );
        let sanitized = sanitize(source).text;
        assert_ne!(
            sanitized.get(start..end),
            Some(expected),
            "fixture must actually diverge from the sanitized slice"
        );
        let text = super::diagnostics_text(source, diagnostics);
        assert!(
            text.contains(&format!("  > {expected:?}")),
            "offending slice must quote the source substring: {text:?}"
        );
    }

    #[test]
    fn slice_uses_source_coordinates_across_crlf_shift() {
        assert_source_coordinate_slice("\r\n\u{E001}", "\u{E001}");
    }

    #[test]
    fn slice_uses_source_coordinates_across_bom_shift() {
        assert_source_coordinate_slice("\u{FEFF}\u{E001}", "\u{E001}");
    }

    #[test]
    fn slice_uses_source_coordinates_across_accent_decomposition() {
        assert_source_coordinate_slice("〔cafe'〕", "〔cafe'〕");
    }

    #[test]
    fn empty_when_no_diagnostics() {
        let doc = Document::new("｜青空《あおぞら》");
        let tree = doc.snapshot();
        let text = super::diagnostics_text(doc.source(), tree.diagnostics());
        assert!(text.is_empty(), "clean parse renders nothing: {text:?}");
    }

    #[test]
    fn renders_code_and_offending_slice() {
        let doc = Document::new("｜《おうめ》");
        let tree = doc.snapshot();
        let diagnostics = tree.diagnostics();
        if diagnostics.is_empty() {
            return;
        }
        let text = super::diagnostics_text(doc.source(), diagnostics);
        assert!(
            text.contains("aozora::") && text.contains('@'),
            "header carries the code + span: {text}"
        );
    }

    #[test]
    fn renders_nonempty_report_for_guaranteed_diagnostic() {
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
        let headers = text.lines().filter(|l| l.contains("] @ ")).count();
        assert_eq!(
            headers,
            diagnostics.len(),
            "one header per diagnostic: {text}"
        );
    }
}
