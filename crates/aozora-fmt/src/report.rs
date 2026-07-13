//! Output rendering: the aggregate [`Outcome`], the JSON `--json` report,
//! and coloured unified diffs.

use std::io::{self, Write};
use std::process::ExitCode;

use anstream::AutoStream;
use anstyle::{AnsiColor, Style};
use anyhow::Result;
use aozora::render::SerializeOptions;
use serde::Serialize;
use similar::{ChangeTag, DiffOp, TextDiff};

use crate::cli::ColorChoice;
use crate::discover::Resolved;
use crate::encoding::Encoding;
use crate::process;

/// The aggregate result of a run. Ordered so folding with `max` keeps the
/// most severe outcome: `Error` > `WouldReformat` > `Ok`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Outcome {
    /// Everything was already formatted (or written / listed) without error.
    Ok,
    /// `--check` found at least one input that would change.
    WouldReformat,
    /// An I/O error, missing path, parser panic, or guard failure occurred.
    Error,
}

impl Outcome {
    /// Map to the documented process exit code (0 / 1 / 2).
    pub(crate) fn exit_code(self) -> ExitCode {
        match self {
            Self::Ok => ExitCode::SUCCESS,
            Self::WouldReformat => ExitCode::from(1),
            Self::Error => ExitCode::from(2),
        }
    }
}

/// Build the stdout stream for coloured output, honouring `--color`. `anstream`
/// strips ANSI when the choice (or TTY detection, for `auto`) says no colour.
///
/// Re-exported from the crate root so the `aozora` CLI's terminal renderers
/// share one TTY/`NO_COLOR` policy with the formatter's diffs.
#[must_use]
pub fn auto_stdout(color: ColorChoice) -> AutoStream<io::Stdout> {
    match color {
        ColorChoice::Auto => AutoStream::auto(io::stdout()),
        ColorChoice::Always => AutoStream::always(io::stdout()),
        ColorChoice::Never => AutoStream::never(io::stdout()),
    }
}

/// Write a coloured unified diff of `old` → `new` under a `label` header.
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
    for group in &diff.grouped_ops(3) {
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

/// 1-based `(old_start, old_len, new_start, new_len)` spanning a hunk group.
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

/// One entry in the `--json` report.
#[derive(Serialize)]
pub(crate) struct JsonFile {
    path: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

impl JsonFile {
    /// An already-formatted file.
    pub(crate) fn ok(path: String) -> Self {
        Self {
            path,
            status: "ok",
            message: None,
        }
    }

    /// A file that `--check` would reformat.
    pub(crate) fn would_reformat(path: String) -> Self {
        Self {
            path,
            status: "would_reformat",
            message: None,
        }
    }

    /// A file that could not be read or formatted.
    pub(crate) fn error(path: String, message: String) -> Self {
        Self {
            path,
            status: "error",
            message: Some(message),
        }
    }
}

/// The `--json` report envelope: the two-key `{ schemaVersion, data }` shape
/// the CLI-local JSON outputs share (matching the wire envelopes' key names and
/// camelCase). `schemaVersion` is a CLI-side counter, distinct from the
/// `aozora::json` wire `SCHEMA_VERSION`.
#[derive(Serialize)]
struct JsonReport {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    data: JsonReportData,
}

/// The `data` payload of the `--json` report: the aggregate `formatted` flag
/// and the per-file statuses.
#[derive(Serialize)]
struct JsonReportData {
    formatted: bool,
    files: Vec<JsonFile>,
}

/// Print the JSON report to stdout. `formatted` is true only when every
/// input was already canonical.
pub(crate) fn emit_json(outcome: Outcome, files: Vec<JsonFile>) -> io::Result<()> {
    let report = JsonReport {
        schema_version: 1,
        data: JsonReportData {
            formatted: outcome == Outcome::Ok,
            files,
        },
    };
    let mut out = io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, &report)?;
    out.write_all(b"\n")
}

/// `--check --json` over a resolved file set: collect every file's status
/// (including discovery errors) into one JSON object and return the outcome.
pub(crate) fn run_check_json(
    encoding: Encoding,
    resolved: &Resolved,
    opts: SerializeOptions,
) -> Result<Outcome> {
    let mut files = Vec::new();
    let mut outcome = Outcome::Ok;
    for err in &resolved.errors {
        files.push(JsonFile::error("<discovery>".to_owned(), err.clone()));
        outcome = Outcome::Error;
    }
    for path in &resolved.files {
        let label = path.display().to_string();
        match process::read_and_format(path, opts, encoding) {
            Ok(fmt) if fmt.changed() => {
                files.push(JsonFile::would_reformat(label));
                outcome = outcome.max(Outcome::WouldReformat);
            }
            Ok(_) => files.push(JsonFile::ok(label)),
            Err(err) => {
                files.push(JsonFile::error(label, format!("{err:#}")));
                outcome = outcome.max(Outcome::Error);
            }
        }
    }
    emit_json(outcome, files)?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Outcome` folds with `max` to keep the most severe result, so the
    /// ordering `Ok < WouldReformat < Error` is load-bearing.
    #[test]
    fn outcome_orders_by_severity() {
        assert!(Outcome::Ok < Outcome::WouldReformat);
        assert!(Outcome::WouldReformat < Outcome::Error);
        assert_eq!(
            Outcome::Ok.max(Outcome::Error).max(Outcome::WouldReformat),
            Outcome::Error,
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
        // Two well-separated edits produce two `@@` groups, exercising
        // `hunk_span`'s min/max accumulation across distinct op groups.
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";
        let new = "X\n2\n3\n4\n5\n6\n7\n8\n9\nY\n";
        let mut out = Vec::new();
        write_diff(&mut out, "f", old, new).expect("diff");
        let text = String::from_utf8(out).expect("utf8");
        // Each hunk header is `@@ -a,b +c,d @@`; count headers via the
        // unambiguous `@@ -` prefix.
        assert_eq!(
            text.matches("@@ -").count(),
            2,
            "two separated edits make two hunks: {text}",
        );
    }

    #[test]
    fn hunk_span_pins_exact_span_header() {
        // A single-line change in the middle of a 10-line file. With three
        // lines of context each side, the hunk covers old/new indices 2..9,
        // so `hunk_span` must emit 1-based start 3 and length 7 on both sides:
        // `@@ -3,7 +3,7 @@`. This pins every field of the returned tuple,
        // and the operands (start 2, end 9) are chosen so `+1`/`*1` disagree
        // and `end - start` differs from `end + start`.
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
        // Only the `---` / `+++` headers, no `@@` hunks.
        assert!(!text.contains("@@"), "no hunks for identical input: {text}");
    }

    #[test]
    fn json_report_uses_the_two_key_data_envelope() {
        // `{ schemaVersion, data:{ formatted, files } }` — the CLI-local shape:
        // `formatted`/`files` live UNDER `data`, and `schemaVersion` is the
        // CLI-side counter `1` (distinct from the wire `SCHEMA_VERSION`).
        let report = JsonReport {
            schema_version: 1,
            data: JsonReportData {
                formatted: false,
                files: vec![JsonFile::would_reformat("a.afm".to_owned())],
            },
        };
        let v = serde_json::to_value(&report).unwrap();
        assert_eq!(v["schemaVersion"], 1, "cli-local counter: {v}");
        assert_eq!(v["data"]["formatted"], false, "formatted nested: {v}");
        assert_eq!(v["data"]["files"][0]["status"], "would_reformat", "{v}");
        // The payload must not leak to the top level.
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
