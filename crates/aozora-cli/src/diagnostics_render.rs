//! Diagnostic rendering for `aozora check`.
//!
//! Three views over the same `&[Diagnostic]`:
//!
//! - **human** — miette's graphical report: the source line, a caret
//!   under the offending span, the label, and the help text. The
//!   default when stderr is a terminal.
//! - **json** — the `aozora::json` diagnostics envelope, byte-identical
//!   to what the WASM / FFI / Python / Extism front doors emit. The
//!   default when stderr is piped, so agents and CI get a stable,
//!   parseable stream without a flag.
//! - **short** — one rustc-style line per diagnostic
//!   (`path:offset: severity[code]: message`), for editors that render
//!   their own snippets.

use std::io::{self, IsTerminal, Write};

use aozora::Document;
use aozora::json;
use aozora::pipeline::lexer::sanitize;
use clap::ValueEnum;
use miette::{NamedSource, Report};

/// How `aozora check` renders diagnostics.
#[derive(Debug, Clone, Copy, Default, ValueEnum, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DiagFormat {
    /// Graphical (`human`) on a terminal, machine (`json`) when piped.
    #[default]
    Auto,
    /// miette graphical report — source snippet + caret + label + help.
    Human,
    /// The `aozora::json` JSON envelope — the cross-binding machine view.
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
    // After the graphical reports, point the reader at `aozora explain
    // <code>` so the obvious next step is one copy-paste away. Human-only:
    // `json` / `short` are machine contracts (ADR-0008) and stay
    // byte-identical, so the hint never reaches them.
    write_explain_hint(&mut stderr, diagnostics)
}

/// Append a one-time `aozora explain <code>` pointer covering the
/// distinct diagnostic codes present, in first-seen order and capped so
/// noisy input does not bury the reports. A no-op when `diagnostics` is
/// empty.
fn write_explain_hint(w: &mut impl Write, diagnostics: &[aozora::Diagnostic]) -> io::Result<()> {
    const MAX_HINTS: usize = 3;
    let mut seen: Vec<&'static str> = Vec::new();
    for diag in diagnostics {
        let short = short_code(diag.code());
        if !seen.contains(&short) {
            seen.push(short);
        }
    }
    if seen.is_empty() {
        return Ok(());
    }
    writeln!(w, "help: run `aozora explain <code>` for details, e.g.")?;
    for code in seen.iter().take(MAX_HINTS) {
        writeln!(w, "      aozora explain {code}")?;
    }
    if seen.len() > MAX_HINTS {
        writeln!(w, "      … and {} more", seen.len() - MAX_HINTS)?;
    }
    Ok(())
}

/// The terminal segment of a `::`-qualified diagnostic code —
/// `aozora::lex::nested_ruby` → `nested_ruby` — which is the short form
/// `aozora explain` accepts.
fn short_code(code: &'static str) -> &'static str {
    code.rsplit_once("::").map_or(code, |(_, tail)| tail)
}

fn render_json(diagnostics: &[aozora::Diagnostic]) -> io::Result<()> {
    writeln!(io::stderr().lock(), "{}", json::diagnostics(diagnostics))
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
