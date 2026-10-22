//! Renders formatter outcomes, JSON reports, and unified diffs.

use std::io::{self, Write};
use std::process::ExitCode;

const INTERNAL_EXIT_CODE: u8 = 3;
const DIFF_CONTEXT_LINES: usize = 3;

use anstream::AutoStream;
use anstyle::{AnsiColor, Style};
use anyhow::Result;
use aozora::SerializeOptions;
use serde::Serialize;
use similar::{ChangeTag, DiffOp, TextDiff};

use crate::fmt::Ctx;
use crate::fmt::cli::ColorChoice;
use crate::fmt::discover::Resolved;
use crate::fmt::process;
use crate::output::{self, StdoutWriter};
use crate::wire::Envelope;
#[cfg(test)]
use aozora::json::SCHEMA_VERSION;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Outcome {
    Ok,
    WouldReformat,
    Diagnostics,
    Error,
    Internal,
}

impl Outcome {
    pub(crate) fn exit_code(self) -> ExitCode {
        match self {
            Self::Ok => ExitCode::SUCCESS,
            Self::WouldReformat | Self::Diagnostics => ExitCode::from(1),
            Self::Error => ExitCode::from(2),
            Self::Internal => ExitCode::from(INTERNAL_EXIT_CODE),
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub(crate) struct FileOutcome {
    pub(crate) outcome: Outcome,
    pub(crate) changed: bool,
}

impl FileOutcome {
    pub(crate) fn new(outcome: Outcome, changed: bool) -> Self {
        Self { outcome, changed }
    }
}

#[must_use]
pub(super) fn auto_stdout(color: ColorChoice) -> StdoutWriter<AutoStream<io::Stdout>> {
    output::guard(match color {
        ColorChoice::Auto => AutoStream::auto(io::stdout()),
        ColorChoice::Always => AutoStream::always(io::stdout()),
        ColorChoice::Never => AutoStream::never(io::stdout()),
    })
}

pub(crate) fn write_diff(
    out: &mut impl Write,
    label: &str,
    old: &str,
    new: &str,
) -> io::Result<()> {
    let header = Style::new().bold();
    let meta = Style::new().fg_color(Some(AnsiColor::Cyan.into()));
    let del = Style::new().fg_color(Some(AnsiColor::Red.into()));
    let ins = Style::new().fg_color(Some(AnsiColor::Green.into()));

    writeln!(out, "{header}--- {label}{header:#}")?;
    writeln!(out, "{header}+++ {label}{header:#}")?;

    let diff = TextDiff::from_lines(old, new);
    for group in &diff.grouped_ops(DIFF_CONTEXT_LINES) {
        let (os, ol, ns, nl) = hunk_span(group);
        writeln!(out, "{meta}@@ -{os},{ol} +{ns},{nl} @@{meta:#}")?;
        for op in group {
            for change in diff.iter_changes(op) {
                let value = change.value();
                let line = value.strip_suffix('\n').unwrap_or(value);
                match change.tag() {
                    ChangeTag::Delete => writeln!(out, "{del}-{line}{del:#}")?,
                    ChangeTag::Insert => writeln!(out, "{ins}+{line}{ins:#}")?,
                    ChangeTag::Equal => writeln!(out, " {line}")?,
                }
            }
        }
    }
    Ok(())
}

/// Returns one-based old and new start positions and lengths for a hunk group.
fn hunk_span(ops: &[DiffOp]) -> (usize, usize, usize, usize) {
    let (mut os, mut ns) = (usize::MAX, usize::MAX);
    let (mut oe, mut ne) = (0_usize, 0_usize);
    for op in ops {
        let (old, new) = (op.old_range(), op.new_range());
        os = os.min(old.start);
        oe = oe.max(old.end);
        ns = ns.min(new.start);
        ne = ne.max(new.end);
    }
    (os + 1, oe - os, ns + 1, ne - ns)
}

#[derive(Serialize)]
pub(crate) struct JsonFile {
    path: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

impl JsonFile {
    pub(crate) fn ok(path: String) -> Self {
        Self {
            path,
            status: "ok",
            message: None,
        }
    }

    pub(crate) fn would_reformat(path: String) -> Self {
        Self {
            path,
            status: "would_reformat",
            message: None,
        }
    }

    pub(crate) fn error(path: String, message: String) -> Self {
        Self {
            path,
            status: "error",
            message: Some(message),
        }
    }
}

#[derive(Serialize)]
struct JsonReportData {
    formatted: bool,
    files: Vec<JsonFile>,
}

pub(crate) fn emit_json(files: Vec<JsonFile>) -> io::Result<()> {
    let formatted = files.iter().all(|file| file.status == "ok");
    let report = Envelope::new(JsonReportData { formatted, files });
    let encoded = serde_json::to_vec_pretty(&report).map_err(io::Error::other)?;
    let mut out = output::stdout();
    out.write_all(&encoded)?;
    out.write_all(b"\n")
}

pub(crate) fn run_check_json(
    ctx: &Ctx,
    resolved: &Resolved,
    opts: SerializeOptions,
) -> Result<Outcome> {
    let mut files = Vec::new();
    let mut outcome = Outcome::Ok;
    for err in &resolved.errors {
        files.push(JsonFile::error("<discovery>".to_owned(), err.clone()));
        outcome = Outcome::Error;
    }
    let mut remaining = resolved.files.as_slice();
    while !remaining.is_empty() {
        let (chunk, rest) = remaining.split_at(process::batch_len(remaining));
        let formatted = process::read_and_format_batch(chunk, opts, ctx.encoding);
        for (path, formatted) in chunk.iter().zip(formatted) {
            let label = path.display().to_string();
            match formatted {
                Ok(fmt) => {
                    let diagnostics =
                        super::diagnostics_outcome(ctx, path, &fmt.old, &fmt.diagnostics)?;
                    if fmt.changed() {
                        files.push(JsonFile::would_reformat(label));
                        outcome = outcome.max(Outcome::WouldReformat).max(diagnostics);
                    } else {
                        files.push(JsonFile::ok(label));
                        outcome = outcome.max(diagnostics);
                    }
                }
                Err(err) => {
                    files.push(JsonFile::error(label, format!("{err:#}")));
                    outcome = outcome.max(Outcome::Error);
                }
            }
        }
        remaining = rest;
    }
    emit_json(files)?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_orders_by_severity() {
        assert!(Outcome::Ok < Outcome::WouldReformat);
        assert!(Outcome::WouldReformat < Outcome::Diagnostics);
        assert!(Outcome::Diagnostics < Outcome::Error);
        assert!(Outcome::Error < Outcome::Internal);
        assert_eq!(
            Outcome::Ok.max(Outcome::Error).max(Outcome::Internal),
            Outcome::Internal,
        );
    }

    #[test]
    fn write_diff_renders_unified_hunk() {
        let mut out = Vec::new();
        write_diff(&mut out, "label", "a\nb\nc\n", "a\nX\nc\n").expect("diff");
        let text = String::from_utf8(out).expect("utf8");
        assert!(text.contains("--- label"), "header: {text}");
        assert!(text.contains("@@"), "hunk marker: {text}");
        assert!(text.contains("-b"), "deletion: {text}");
        assert!(text.contains("+X"), "insertion: {text}");
        assert!(text.contains(" a"), "context line: {text}");
    }

    #[test]
    fn write_diff_spans_multiple_hunks() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";
        let new = "X\n2\n3\n4\n5\n6\n7\n8\n9\nY\n";
        let mut out = Vec::new();
        write_diff(&mut out, "f", old, new).expect("diff");
        let text = String::from_utf8(out).expect("utf8");
        assert_eq!(
            text.matches("@@ -").count(),
            2,
            "two separated edits make two hunks: {text}",
        );
    }

    #[test]
    fn hunk_span_pins_exact_span_header() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
        let new = "a\nb\nc\nd\ne\nX\ng\nh\ni\nj\n";
        let mut out = Vec::new();
        write_diff(&mut out, "f", old, new).expect("diff");
        let text = String::from_utf8(out).expect("utf8");
        assert_eq!(
            text.matches("@@ -").count(),
            1,
            "one middle edit makes exactly one hunk: {text}",
        );
        assert!(
            text.contains("@@ -3,7 +3,7 @@"),
            "hunk header must pin start 3 and length 7 on both sides: {text}",
        );
    }

    #[test]
    fn write_diff_empty_when_identical() {
        let mut out = Vec::new();
        write_diff(&mut out, "f", "same\n", "same\n").expect("diff");
        let text = String::from_utf8(out).expect("utf8");
        assert!(!text.contains("@@"), "no hunks for identical input: {text}");
    }

    #[test]
    fn json_report_uses_the_two_key_data_envelope() {
        let report = Envelope::new(JsonReportData {
            formatted: false,
            files: vec![JsonFile::would_reformat("a.afm".to_owned())],
        });
        let v = serde_json::to_value(&report).unwrap();
        assert_eq!(v["schemaVersion"], SCHEMA_VERSION, "wire version: {v}");
        assert_eq!(v["data"]["formatted"], false, "formatted nested: {v}");
        assert_eq!(v["data"]["files"][0]["status"], "would_reformat", "{v}");
        assert!(
            v.get("formatted").is_none(),
            "formatted must be nested: {v}"
        );
        assert!(v.get("files").is_none(), "files must be nested: {v}");
        assert!(v.get("version").is_none(), "old `version` key gone: {v}");
    }

    #[test]
    fn json_file_variants_serialise_with_expected_shape() {
        let ok = serde_json::to_value(JsonFile::ok("a.afm".to_owned())).unwrap();
        assert_eq!(ok["status"], "ok");
        assert!(ok.get("message").is_none(), "ok omits message: {ok}");

        let would = serde_json::to_value(JsonFile::would_reformat("b.afm".to_owned())).unwrap();
        assert_eq!(would["status"], "would_reformat");
        assert!(would.get("message").is_none());

        let err =
            serde_json::to_value(JsonFile::error("c.afm".to_owned(), "boom".to_owned())).unwrap();
        assert_eq!(err["status"], "error");
        assert_eq!(err["message"], "boom");
        assert_eq!(err["path"], "c.afm");
    }
}
