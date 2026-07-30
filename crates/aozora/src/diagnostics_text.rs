#![expect(
    clippy::expect_used,
    reason = "diagnostic rendering writes into an infallible String buffer"
)]

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

use crate::Diagnostic;

/// Render `diagnostics` over `source` as a plain-text report.
///
/// One block per diagnostic — a `<severity> [<code>] @ <start>..<end>:
/// <message>` header followed by the offending source slice — joined by
/// newlines. Returns the empty string when `diagnostics` is empty.
///
/// Spans are in *source* coordinates — [`Snapshot::diagnostics`] maps
/// them back onto the original bytes — so the offending slice is cut
/// from `source` directly. A span that falls out of range or lands
/// mid-codepoint degrades to no slice rather than panicking.
///
/// [`Snapshot::diagnostics`]: crate::Snapshot::diagnostics
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
    for diag in diagnostics {
        let span = diag.span();
        let (start, end) = (span.start as usize, span.end as usize);
        writeln!(
            out,
            "{severity} [{code}] @ {start}..{end}: {diag}",
            severity = diag.severity().as_json_str(),
            code = diag.code(),
        )?;
        // The span is in source coordinates, so the offending slice is
        // cut from the original `source`. `str::get` yields `None` for an
        // out-of-range or non-char-boundary span, degrading to no slice
        // rather than panicking.
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

    /// Assert that the offending slice `diagnostics_text` quotes for
    /// `source` is cut from the *original* source at the source-coordinate
    /// span — not from a re-sanitized copy, which for BOM / CRLF /
    /// `〔…〕`-accent inputs shifts the text or falls out of range.
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
        // The sanitized copy the old code sliced disagrees at this span —
        // out of range (BOM/CRLF) or the decomposed text (accent) — so a
        // render that quotes `expected` proves the slice came from source.
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
        // `\r\n` folds to `\n`, so the sentinel sits one byte later in
        // source than in sanitized text; the old sanitize-and-slice path
        // cut the sanitized copy at the source span and lost the slice.
        assert_source_coordinate_slice("\r\n\u{E001}", "\u{E001}");
    }

    #[test]
    fn slice_uses_source_coordinates_across_bom_shift() {
        // A stripped leading BOM (3 bytes) shifts every later offset; the
        // source span lands out of range of the shorter sanitized text.
        assert_source_coordinate_slice("\u{FEFF}\u{E001}", "\u{E001}");
    }

    #[test]
    fn slice_uses_source_coordinates_across_accent_decomposition() {
        // `〔cafe'〕` decomposes to `〔café〕`: same byte width, different
        // content, so slicing the sanitized copy would quote the
        // decomposed form instead of the source the author typed.
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
