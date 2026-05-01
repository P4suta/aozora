//! End-to-end smoke tests for `aozora kinds` / `aozora schema` /
//! `aozora explain` (Phase L3).
//!
//! Mirrors `smoke.rs` in spawning the *built* binary via
//! `CARGO_BIN_EXE_aozora` so argv → clap → introspect dispatch is
//! exercised end-to-end. The library tests in `aozora-spec` /
//! `aozora-syntax` already pin every enum variant; these tests pin
//! the *CLI shape* — that the subcommand exists, the columns appear,
//! the schema parses, and unknown explain tags surface a hint.

use std::process::{Command, ExitStatus, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_aozora");

fn run(args: &[&str]) -> (ExitStatus, String, String) {
    let output = Command::new(BIN)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn aozora");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    (output.status, stdout, stderr)
}

// ---------------------------------------------------------------------
// `aozora kinds`
// ---------------------------------------------------------------------

#[test]
fn kinds_lists_every_enum_section() {
    let (status, stdout, stderr) = run(&["kinds"]);
    assert!(status.success(), "kinds failed: {stderr:?}");
    for section in [
        "NodeKind",
        "PairKind",
        "Severity",
        "DiagnosticSource",
        "Sentinel",
        "InternalCheckCode",
    ] {
        assert!(
            stdout.contains(section),
            "kinds output missing {section} section: {stdout:?}",
        );
    }
}

#[test]
fn kinds_lists_concrete_node_tags() {
    let (status, stdout, _) = run(&["kinds"]);
    assert!(status.success());
    // Spot-check tags that span the camelCase / non-ascii lookup paths.
    for tag in ["ruby", "doubleRuby", "containerOpen", "containerClose"] {
        assert!(stdout.contains(tag), "kinds missing tag {tag}: {stdout:?}");
    }
}

// ---------------------------------------------------------------------
// `aozora schema`
// ---------------------------------------------------------------------

#[test]
fn schema_diagnostics_emits_valid_json() {
    let (status, stdout, stderr) = run(&["schema", "diagnostics"]);
    assert!(status.success(), "schema diagnostics failed: {stderr:?}");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("schema output must be valid JSON");
    assert_eq!(
        parsed["title"].as_str(),
        Some("AozoraDiagnosticsEnvelope"),
        "schema title mismatch: {parsed:?}",
    );
}

#[test]
fn schema_each_envelope_succeeds() {
    for which in ["diagnostics", "nodes", "pairs", "container-pairs"] {
        let (status, stdout, stderr) = run(&["schema", which]);
        assert!(status.success(), "schema {which} failed: {stderr:?}");
        assert!(
            serde_json::from_str::<serde_json::Value>(&stdout).is_ok(),
            "schema {which} output is not valid JSON",
        );
    }
}

// ---------------------------------------------------------------------
// `aozora explain`
// ---------------------------------------------------------------------

#[test]
fn explain_known_kind_succeeds() {
    let (status, stdout, _) = run(&["explain", "ruby"]);
    assert!(status.success(), "explain ruby must succeed");
    assert!(stdout.contains("NodeKind::Ruby"), "missing tag: {stdout:?}");
    assert!(
        stdout.contains("Phase O1"),
        "missing forward pointer: {stdout:?}"
    );
}

#[test]
fn explain_camelcase_tag_succeeds() {
    let (status, stdout, _) = run(&["explain", "doubleRuby"]);
    assert!(status.success(), "explain doubleRuby must succeed");
    assert!(stdout.contains("DoubleRuby"), "missing tag: {stdout:?}");
}

#[test]
fn explain_unknown_kind_fails_with_hint() {
    let (status, _, stderr) = run(&["explain", "bogus"]);
    assert!(!status.success(), "unknown kind must exit non-zero");
    assert!(
        stderr.contains("aozora kinds"),
        "expected hint pointing at `aozora kinds`: {stderr:?}",
    );
}
