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
use aozora_pipeline::lexer::sanitize;
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
        self.resolve(io::stderr().is_terminal())
    }

    /// Pure decision seam for [`resolved`](Self::resolved): `Auto` becomes
    /// `Human` on a terminal and `Json` otherwise; concrete formats pass
    /// through unchanged.
    fn resolve(self, is_terminal: bool) -> Self {
        match self {
            Self::Auto if is_terminal => Self::Human,
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
    // Diagnostic spans live in SANITIZED coordinates: the sanitize stage
    // strips the BOM, folds CRLF→LF, and decomposes 〔…〕 accent digraphs — each of
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
    if let Some(hint) = explain_hint(diagnostics.iter().map(aozora::Diagnostic::code)) {
        w.write_all(hint.as_bytes())?;
    }
    Ok(())
}

/// Pure formatting seam for [`write_explain_hint`]: build the hint text
/// for the distinct short codes among `codes` (first-seen order, capped at
/// `MAX_HINTS` with an `… and N more` tail), or `None` when there are none.
fn explain_hint(codes: impl Iterator<Item = &'static str>) -> Option<String> {
    use std::fmt::Write as _;

    const MAX_HINTS: usize = 3;
    let mut seen: Vec<&'static str> = Vec::new();
    for code in codes {
        let short = short_code(code);
        if !seen.contains(&short) {
            seen.push(short);
        }
    }
    if seen.is_empty() {
        return None;
    }
    let mut out = String::new();
    writeln!(out, "help: run `aozora explain <code>` for details, e.g.")
        .expect("writing to a String is infallible");
    for code in seen.iter().take(MAX_HINTS) {
        writeln!(out, "      aozora explain {code}").expect("writing to a String is infallible");
    }
    if seen.len() > MAX_HINTS {
        writeln!(out, "      … and {} more", seen.len() - MAX_HINTS)
            .expect("writing to a String is infallible");
    }
    Some(out)
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
            diag.severity().as_json_str(),
            diag.code(),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // `resolve` is the pure TTY-decision seam behind `resolved()`. On a
    // terminal `Auto` renders `Human`; piped, it renders `Json`. Driving
    // both branches kills the `is_terminal` guard being pinned to `false`.
    #[test]
    fn resolve_auto_on_terminal_is_human() {
        assert!(matches!(DiagFormat::Auto.resolve(true), DiagFormat::Human));
    }

    #[test]
    fn resolve_auto_when_piped_is_json() {
        assert!(matches!(DiagFormat::Auto.resolve(false), DiagFormat::Json));
    }

    // Concrete formats ignore the terminal flag and pass straight through,
    // regardless of which side of the guard we are on.
    #[test]
    fn resolve_concrete_formats_pass_through() {
        assert!(matches!(DiagFormat::Human.resolve(true), DiagFormat::Human));
        assert!(matches!(
            DiagFormat::Human.resolve(false),
            DiagFormat::Human
        ));
        assert!(matches!(DiagFormat::Json.resolve(true), DiagFormat::Json));
        assert!(matches!(DiagFormat::Json.resolve(false), DiagFormat::Json));
        assert!(matches!(DiagFormat::Short.resolve(true), DiagFormat::Short));
        assert!(matches!(
            DiagFormat::Short.resolve(false),
            DiagFormat::Short
        ));
    }

    #[test]
    fn explain_hint_empty_is_none() {
        use std::iter::empty;
        assert_eq!(explain_hint(empty()), None);
    }

    // Deduped to distinct short codes, in first-seen order.
    #[test]
    fn explain_hint_dedups_by_short_code() {
        let codes = ["aozora::lex::foo", "aozora::parse::foo", "aozora::lex::bar"];
        let hint = explain_hint(codes.into_iter()).expect("non-empty");
        assert_eq!(
            hint,
            "help: run `aozora explain <code>` for details, e.g.\n\
             \u{20}     aozora explain foo\n\
             \u{20}     aozora explain bar\n",
        );
    }

    // Exactly `MAX_HINTS` (3) distinct codes: every code is listed and there
    // is NO `… and N more` tail. `seen.len() > MAX_HINTS` is `3 > 3` == false;
    // mutating `>` to `==` or `>=` would make `3 <op> 3` true and emit a
    // spurious `… and 0 more`, so asserting the tail's absence kills both.
    #[test]
    fn explain_hint_exactly_max_has_no_more_tail() {
        let codes = ["aozora::a::x", "aozora::b::y", "aozora::c::z"];
        let hint = explain_hint(codes.into_iter()).expect("non-empty");
        assert_eq!(
            hint,
            "help: run `aozora explain <code>` for details, e.g.\n\
             \u{20}     aozora explain x\n\
             \u{20}     aozora explain y\n\
             \u{20}     aozora explain z\n",
        );
        assert!(!hint.contains("more"));
    }

    // One over the cap (4 distinct codes): only the first three are listed
    // and a `… and 1 more` tail appears. `seen.len() > MAX_HINTS` is
    // `4 > 3` == true; mutating `>` to `<` or `==` makes `4 <op> 3` false and
    // drops the tail, so asserting the tail's presence kills both.
    #[test]
    fn explain_hint_over_max_shows_more_tail() {
        let codes = [
            "aozora::a::x",
            "aozora::b::y",
            "aozora::c::z",
            "aozora::d::w",
        ];
        let hint = explain_hint(codes.into_iter()).expect("non-empty");
        assert_eq!(
            hint,
            "help: run `aozora explain <code>` for details, e.g.\n\
             \u{20}     aozora explain x\n\
             \u{20}     aozora explain y\n\
             \u{20}     aozora explain z\n\
             \u{20}     … and 1 more\n",
        );
    }
}
