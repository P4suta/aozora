//! Integration coverage for `src/diagnostics_render.rs`.
//!
//! The three diagnostic views (`human` / `json` / `short`) write to
//! the process's stderr, so they are exercised by spawning the real
//! binary with `--diagnostic-format` and capturing stderr — the same
//! `Command` + `CARGO_BIN_EXE_aozora` pattern as `smoke.rs` /
//! `snapshot_cli.rs`. Each test pins a structural property of the
//! rendered output so a regression in the formatter lands as a
//! review diff, not a silent drift.
//!
//! Diagnostic spans live in SANITIZED coordinates; the inputs here use
//! only LF and no BOM so source / sanitized byte offsets coincide and
//! the assertions stay deterministic.

use std::io::Write;
use std::process::{Command, Stdio};

use tempfile::Builder;

const BIN: &str = env!("CARGO_BIN_EXE_aozora");

/// Run `aozora <args>` feeding `stdin`; return `(stdout, stderr)`.
/// Both pipes are captured, so the `Auto` diagnostic format resolves
/// to the machine (`json`) view (stderr is not a TTY).
fn run(args: &[&str], stdin: &[u8]) -> (String, String) {
    let mut child = Command::new(BIN)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aozora");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(stdin)
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait for aozora");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

// A single lexer PUA sentinel (U+E001) → a `source_contains_pua`
// warning. Bytes: EE 80 81 in UTF-8, at offset 1.
const ONE_PUA: &[u8] = b"a\xee\x80\x81b";
// Two distinct sentinels (U+E001, U+E002) → two diagnostics, so the
// per-diagnostic loops in every formatter run more than once.
const TWO_PUA: &[u8] = b"a\xee\x80\x81b\xee\x80\x82c";

// ---------------------------------------------------------------------
// short
// ---------------------------------------------------------------------

#[test]
fn short_format_renders_rustc_style_line() {
    let (_, stderr) = run(&["check", "--diagnostic-format", "short"], ONE_PUA);
    // `path:offset: severity[code]: message`
    assert!(
        stderr.contains("<stdin>:1: warning[aozora::lex::source_contains_pua]:"),
        "short line missing path/offset/severity/code prefix: {stderr:?}"
    );
    assert!(
        stderr.contains("source contains lexer PUA sentinel"),
        "short line missing the message: {stderr:?}"
    );
}

#[test]
fn short_format_emits_one_line_per_diagnostic() {
    let (_, stderr) = run(&["check", "--diagnostic-format", "short"], TWO_PUA);
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(
        lines.len(),
        2,
        "two sentinels → two short lines: {stderr:?}"
    );
    for line in &lines {
        assert!(
            line.contains("warning["),
            "each line carries the severity tag: {line:?}"
        );
    }
}

#[test]
fn short_format_uses_the_file_path() {
    let mut file = Builder::new()
        .prefix("aozora-diag-render-")
        .suffix(".txt")
        .tempfile()
        .expect("temp file");
    file.write_all(ONE_PUA).expect("write temp input");
    file.flush().expect("flush temp input");
    let path = file.path().to_owned();
    let path_str = path.to_str().expect("utf-8 temp path");
    let (_, stderr) = run(&["check", "--diagnostic-format", "short", path_str], &[]);
    assert!(
        stderr.contains(path_str),
        "short line should name the input file path: {stderr:?}"
    );
}

#[test]
fn short_format_renders_note_severity() {
    // 〔e^〕 accent digraph → Phase 0 decomposition `note` diagnostic.
    let (_, stderr) = run(
        &["check", "--diagnostic-format", "short"],
        "〔e^〕".as_bytes(),
    );
    assert!(
        stderr.contains("note[aozora::lex::accent_decomposition_applied]:"),
        "accent decomposition renders as a `note`: {stderr:?}"
    );
}

#[test]
fn short_format_renders_error_severity() {
    // A ruby reading carrying a nested ruby → `nested_ruby` error.
    let (_, stderr) = run(
        &["check", "--diagnostic-format", "short"],
        "｜青《あ｜お《く》》".as_bytes(),
    );
    assert!(
        stderr.contains("error[aozora::lex::nested_ruby]:"),
        "nested ruby renders as an `error`: {stderr:?}"
    );
}

// ---------------------------------------------------------------------
// json
// ---------------------------------------------------------------------

#[test]
fn json_format_emits_wire_envelope() {
    let (_, stderr) = run(&["check", "--diagnostic-format", "json"], ONE_PUA);
    let value: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("json diagnostics envelope parses");
    assert_eq!(
        value["schema_version"], 1,
        "wire envelope carries schema_version 1: {stderr:?}"
    );
    let data = value["data"]
        .as_array()
        .expect("data is an array of diagnostics");
    assert_eq!(data.len(), 1, "one diagnostic in the envelope: {stderr:?}");
    assert_eq!(
        data[0]["kind"], "source_contains_pua",
        "diagnostic kind round-trips: {stderr:?}"
    );
}

#[test]
fn json_format_carries_every_diagnostic() {
    let (_, stderr) = run(&["check", "--diagnostic-format", "json"], TWO_PUA);
    let value: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("json envelope parses");
    assert_eq!(
        value["data"].as_array().expect("data array").len(),
        2,
        "both sentinels appear in the JSON envelope: {stderr:?}"
    );
}

#[test]
fn auto_format_resolves_to_json_when_stderr_is_piped() {
    // No `--diagnostic-format`: `Auto` collapses to `json` because the
    // captured stderr is not a terminal.
    let (_, stderr) = run(&["check"], ONE_PUA);
    let value: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("auto → json envelope parses");
    assert_eq!(
        value["schema_version"], 1,
        "auto resolves to the json envelope off a TTY: {stderr:?}"
    );
}

// ---------------------------------------------------------------------
// human
// ---------------------------------------------------------------------

#[test]
fn human_format_renders_graphical_report() {
    let (_, stderr) = run(&["check", "--diagnostic-format", "human"], ONE_PUA);
    // miette's graphical report carries the code, the message, a source
    // snippet line and a caret label.
    assert!(
        stderr.contains("source_contains_pua"),
        "human report names the diagnostic code: {stderr:?}"
    );
    assert!(
        stderr.contains("source contains lexer PUA sentinel"),
        "human report carries the message: {stderr:?}"
    );
    assert!(
        stderr.contains("here"),
        "human report draws the caret label: {stderr:?}"
    );
}

#[test]
fn human_format_renders_a_report_per_diagnostic() {
    let (_, stderr) = run(&["check", "--diagnostic-format", "human"], TWO_PUA);
    let occurrences = stderr.matches("source_contains_pua").count();
    assert_eq!(
        occurrences, 2,
        "one graphical report per diagnostic: {stderr:?}"
    );
}

// ---------------------------------------------------------------------
// no diagnostics → render is never reached, stderr stays empty
// ---------------------------------------------------------------------

#[test]
fn clean_input_writes_nothing_for_any_format() {
    for fmt in ["short", "json", "human", "auto"] {
        let (_, stderr) = run(
            &["check", "--diagnostic-format", fmt],
            "｜青梅《おうめ》\n".as_bytes(),
        );
        assert!(
            stderr.is_empty(),
            "clean input → empty stderr for format {fmt:?}: {stderr:?}"
        );
    }
}
