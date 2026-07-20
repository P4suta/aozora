//! Integration coverage for `src/diagnostics_render.rs`.
//!
//! The three diagnostic views (`human` / `json` / `short`) write to
//! the process's stderr, so they are exercised by spawning the real
//! binary with `--format` and capturing stderr — the same
//! `Command` + `CARGO_BIN_EXE_aozora` pattern as `smoke.rs` /
//! `snapshot_cli.rs`. Each test pins a structural property of the
//! rendered output so a regression in the formatter lands as a
//! review diff, not a silent drift.
//!
//! Diagnostic spans live in original-source UTF-8 byte coordinates.

use std::io::Write;
use std::process::Stdio;

use aozora::json::SCHEMA_VERSION;
use tempfile::Builder;

mod common;

/// Run `aozora <args>` feeding `stdin`; return `(stdout, stderr)`.
/// Both pipes are captured, so the `Auto` diagnostic format resolves
/// to the machine (`json`) view (stderr is not a TTY).
fn run(args: &[&str], stdin: &[u8]) -> (String, String) {
    let mut child = common::hermetic_command()
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
    let (_, stderr) = run(&["check", "--format", "short"], ONE_PUA);
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
    let (_, stderr) = run(&["check", "--format", "short"], TWO_PUA);
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
    let (_, stderr) = run(&["check", "--format", "short", path_str], &[]);
    assert!(
        stderr.contains(path_str),
        "short line should name the input file path: {stderr:?}"
    );
}

#[test]
fn short_format_renders_note_severity() {
    // 〔e^〕 accent digraph → sanitize-stage decomposition `note` diagnostic.
    let (_, stderr) = run(&["check", "--format", "short"], "〔e^〕".as_bytes());
    assert!(
        stderr.contains("note[aozora::lex::accent_decomposition_applied]:"),
        "accent decomposition renders as a `note`: {stderr:?}"
    );
}

#[test]
fn short_format_renders_error_severity() {
    // A ruby reading carrying a nested ruby → `nested_ruby` error.
    let (_, stderr) = run(
        &["check", "--format", "short"],
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
    let (_, stderr) = run(&["check", "--format", "json"], ONE_PUA);
    let value: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("json diagnostics envelope parses");
    assert_eq!(
        value["schemaVersion"], SCHEMA_VERSION,
        "wire envelope carries the core schema version: {stderr:?}"
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
    let (_, stderr) = run(&["check", "--format", "json"], TWO_PUA);
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
    // No `--format`: `Auto` collapses to `json` because the
    // captured stderr is not a terminal.
    let (_, stderr) = run(&["check"], ONE_PUA);
    let value: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("auto → json envelope parses");
    assert_eq!(
        value["schemaVersion"], SCHEMA_VERSION,
        "auto resolves to the json envelope off a TTY: {stderr:?}"
    );
}

// ---------------------------------------------------------------------
// human
// ---------------------------------------------------------------------

#[test]
fn human_format_renders_graphical_report() {
    let (_, stderr) = run(&["check", "--format", "human"], ONE_PUA);
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
fn human_format_uses_original_source_coordinates() {
    let source = "\u{FEFF}first\r\nabc\u{E001}def";
    let (_, stderr) = run(
        &["check", "--format", "human", "--encoding", "utf8"],
        source.as_bytes(),
    );
    assert!(
        stderr.contains("abc\u{E001}def"),
        "human report must attach the original CRLF/BOM source: {stderr:?}",
    );
    assert!(
        stderr.contains("source_contains_pua"),
        "the source-coordinate diagnostic must render: {stderr:?}",
    );
}

#[test]
fn human_format_renders_a_report_per_diagnostic() {
    let (_, stderr) = run(&["check", "--format", "human"], TWO_PUA);
    // One graphical report per diagnostic. Exclude the explain-hint
    // footer, which names the (deduped) code once more.
    let occurrences = stderr
        .lines()
        .filter(|line| !line.contains("aozora explain"))
        .filter(|line| line.contains("source_contains_pua"))
        .count();
    assert_eq!(
        occurrences, 2,
        "one graphical report per diagnostic: {stderr:?}"
    );
}

#[test]
fn human_format_appends_explain_hint() {
    let (_, stderr) = run(&["check", "--format", "human"], ONE_PUA);
    assert!(
        stderr.contains("aozora explain source_contains_pua"),
        "human report points the reader at `aozora explain <code>`: {stderr:?}"
    );
}

#[test]
fn human_explain_hint_dedupes_repeated_codes() {
    // TWO_PUA fires two `source_contains_pua` diagnostics; the hint
    // names the code exactly once.
    let (_, stderr) = run(&["check", "--format", "human"], TWO_PUA);
    assert_eq!(
        stderr.matches("aozora explain source_contains_pua").count(),
        1,
        "the explain hint dedupes repeated codes: {stderr:?}"
    );
}

#[test]
fn explain_hint_absent_from_machine_formats() {
    // The hint is human-only; `json` / `short` are machine contracts
    // (ADR-0008) and must stay byte-identical — no hint.
    for fmt in ["json", "short"] {
        let (_, stderr) = run(&["check", "--format", fmt], ONE_PUA);
        assert!(
            !stderr.contains("aozora explain"),
            "format {fmt:?} must not carry the explain hint: {stderr:?}"
        );
    }
}

// ---------------------------------------------------------------------
// i18n: the human footer localizes; the machine axis never does
// ---------------------------------------------------------------------

#[test]
fn explain_hint_header_is_english_by_default() {
    // The harness pins `AOZORA_LANG=en`, so the human footer header is English.
    let (_, stderr) = run(&["check", "--format", "human"], ONE_PUA);
    assert!(
        stderr.contains("help: run `aozora explain <code>` for details, e.g."),
        "english footer header: {stderr:?}"
    );
}

#[test]
fn explain_hint_header_localizes_with_lang() {
    // `--lang` swaps the human footer header (the per-code command lines stay
    // literal). It outranks the pinned `AOZORA_LANG=en`.
    let (_, ja) = run(&["check", "--format", "human", "--lang", "ja"], ONE_PUA);
    assert!(
        ja.contains("ヒント: 詳細は `aozora explain <code>` を実行。例:"),
        "japanese footer header: {ja:?}"
    );
    assert!(
        ja.contains("aozora explain source_contains_pua"),
        "the per-code command line stays literal under --lang ja: {ja:?}"
    );

    let (_, zh) = run(&["check", "--format", "human", "--lang", "zh"], ONE_PUA);
    assert!(
        zh.contains("提示: 运行 `aozora explain <code>` 查看详情，例如:"),
        "chinese footer header: {zh:?}"
    );
}

#[test]
fn human_report_headline_localizes_but_english_keeps_the_display() {
    // English (the pinned default) keeps the byte-stable `#[error]` Display as
    // the report headline.
    let (_, en) = run(&["check", "--format", "human", "--lang", "en"], ONE_PUA);
    assert!(
        en.contains("source contains lexer PUA sentinel"),
        "en headline is the #[error] Display: {en:?}"
    );

    // `--lang ja` / `zh` substitute the localized title as the headline via the
    // thin adapter — the English Display sentence must NOT appear as a headline.
    let (_, ja) = run(&["check", "--format", "human", "--lang", "ja"], ONE_PUA);
    assert!(
        ja.contains("私用領域文字がソースに紛れ込んでいる"),
        "ja headline is the localized title: {ja:?}"
    );
    assert!(
        !ja.contains("source contains lexer PUA sentinel"),
        "ja must not show the English Display headline: {ja:?}"
    );
    let (_, zh) = run(&["check", "--format", "human", "--lang", "zh"], ONE_PUA);
    assert!(
        zh.contains("源文本中混入了私用区字符"),
        "zh headline is the localized title: {zh:?}"
    );

    // The machine axis inside the human report — the dotted code and the docs
    // URL — is language-invariant across all three.
    for out in [&en, &ja, &zh] {
        assert!(
            out.contains("aozora::lex::source_contains_pua"),
            "code present in every language: {out:?}"
        );
    }
}

#[test]
fn machine_formats_are_byte_identical_across_languages() {
    // The core correctness invariant of the i18n work: json / short output is
    // an English-stable contract, byte-for-byte the same under any `--lang`.
    for fmt in ["json", "short"] {
        let (_, en) = run(&["check", "--format", fmt, "--lang", "en"], TWO_PUA);
        let (_, ja) = run(&["check", "--format", fmt, "--lang", "ja"], TWO_PUA);
        let (_, zh) = run(&["check", "--format", fmt, "--lang", "zh"], TWO_PUA);
        assert_eq!(en, ja, "format {fmt:?}: en and ja bytes differ");
        assert_eq!(en, zh, "format {fmt:?}: en and zh bytes differ");
    }
}

// ---------------------------------------------------------------------
// no diagnostics → render is never reached, stderr stays empty
// ---------------------------------------------------------------------

#[test]
fn clean_input_writes_nothing_for_any_format() {
    for fmt in ["short", "json", "human", "auto"] {
        let (_, stderr) = run(&["check", "--format", fmt], "｜青梅《おうめ》\n".as_bytes());
        assert!(
            stderr.is_empty(),
            "clean input → empty stderr for format {fmt:?}: {stderr:?}"
        );
    }
}
