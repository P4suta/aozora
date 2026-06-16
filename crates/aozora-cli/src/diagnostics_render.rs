//! Diagnostic rendering for `aozora check`.
//!
//! Three views over the same `&[Diagnostic]`:
//!
//! - **human** — miette's graphical report: the source line, a caret
//!   under the offending span, the label, and the help text. The
//!   default when stderr is a terminal.
//! - **json** — the `aozora::wire` diagnostics envelope, byte-identical
//!   to what the WASM / FFI / Python / Extism front doors emit. The
//!   default when stderr is piped, so agents and CI get a stable,
//!   parseable stream without a flag.
//! - **short** — one rustc-style line per diagnostic
//!   (`path:offset: severity[code]: message`), for editors that render
//!   their own snippets.

use std::io::{self, IsTerminal, Write};

use aozora::Document;
use aozora::pipeline::lexer::sanitize;
use aozora::wire::serialize_diagnostics;
use clap::ValueEnum;
use miette::{NamedSource, Report};

/// How `aozora check` renders diagnostics.
#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub(crate) enum DiagFormat {
    /// Graphical (`human`) on a terminal, machine (`json`) when piped.
    #[default]
    Auto,
    /// miette graphical report — source snippet + caret + label + help.
    Human,
    /// The `aozora::wire` JSON envelope — the cross-binding machine view.
    Json,
    /// One grep-able line per diagnostic: `path:offset: severity[code]: msg`.
    Short,
}

impl DiagFormat {
    /// Collapse `Auto` to a concrete view based on whether stderr is a TTY.
    fn resolved(self) -> Self {
        match self {
            Self::Auto if io::stderr().is_terminal() => Self::Human,
            Self::Auto => Self::Json,
            other => other,
        }
    }
}

/// Render `diagnostics` (belonging to `doc`) to stderr in `format`.
pub(crate) fn render(
    format: DiagFormat,
    path: &str,
    doc: &Document,
    diagnostics: &[aozora::Diagnostic],
) -> io::Result<()> {
    match format.resolved() {
        // `resolved()` never returns `Auto`, but match exhaustively.
        DiagFormat::Human | DiagFormat::Auto => render_human(path, doc, diagnostics),
        DiagFormat::Json => render_json(diagnostics),
        DiagFormat::Short => render_short(path, diagnostics),
    }
}

fn render_human(path: &str, doc: &Document, diagnostics: &[aozora::Diagnostic]) -> io::Result<()> {
    // Diagnostic spans live in SANITIZED coordinates: Phase 0 strips the
    // BOM, folds CRLF→LF, and decomposes 〔…〕 accent digraphs — each of
    // which shifts byte offsets. Aozora Bunko files ship as CRLF, so
    // attaching the *raw* bytes would slide every caret right by the
    // number of preceding line breaks. Re-derive the sanitized text (the
    // exact bytes the lexer spanned into) and attach that instead.
    let sanitized = sanitize(doc.source()).text;
    let mut stderr = io::stderr().lock();
    for diag in diagnostics {
        let report = Report::new(diag.clone())
            .with_source_code(NamedSource::new(path, sanitized.to_string()));
        // With miette's `fancy` feature, `{:?}` renders the graphical report.
        writeln!(stderr, "{report:?}")?;
    }
    Ok(())
}

fn render_json(diagnostics: &[aozora::Diagnostic]) -> io::Result<()> {
    writeln!(
        io::stderr().lock(),
        "{}",
        serialize_diagnostics(diagnostics)
    )
}

fn render_short(path: &str, diagnostics: &[aozora::Diagnostic]) -> io::Result<()> {
    let mut stderr = io::stderr().lock();
    for diag in diagnostics {
        let span = diag.span();
        writeln!(
            stderr,
            "{path}:{}: {}[{}]: {diag}",
            span.start,
            diag.severity().as_wire_str(),
            diag.code(),
        )?;
    }
    Ok(())
}
