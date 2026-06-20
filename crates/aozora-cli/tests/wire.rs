//! End-to-end tests for `aozora wire <kind>` — the document-data
//! projection of the shared `aozora::wire` envelope.
//!
//! These verify the argv / dispatch / stdin / file plumbing and that
//! the `{ "schema_version": 1, "data": [ … ] }` envelope reaches
//! stdout. The byte-level shape of each envelope is pinned by the unit
//! tests in `aozora::wire`; here we only confirm the CLI surfaces it
//! (and that `slugs` needs no input while `gaiji` resolves references).
//!
//! Pure stdlib (mirrors `smoke.rs`) so the test crate stays dep-light.

use std::io::Write;
use std::process::{Command, ExitStatus, Stdio};

use tempfile::NamedTempFile;

const BIN: &str = env!("CARGO_BIN_EXE_aozora");

/// Every wire envelope opens with this versioned header; asserting the
/// literal prefix is a structural check that needs no JSON parser in
/// the (deliberately dep-light) test crate.
const ENVELOPE_PREFIX: &str = r#"{"schema_version":1,"#;

fn run(args: &[&str], stdin: Option<&str>) -> (ExitStatus, String, String) {
    let mut cmd = Command::new(BIN);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    if stdin.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let mut child = cmd.spawn().expect("spawn aozora");
    if let Some(s) = stdin {
        child
            .stdin
            .as_mut()
            .expect("piped stdin")
            .write_all(s.as_bytes())
            .expect("write stdin");
    }
    let output = child.wait_with_output().expect("wait");
    (
        output.status,
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn write_temp(contents: &str) -> NamedTempFile {
    let mut f = tempfile::Builder::new()
        .prefix("aozora-wire-test-")
        .suffix(".txt")
        .tempfile()
        .expect("temp file");
    f.write_all(contents.as_bytes()).expect("write temp");
    f.flush().expect("flush temp");
    f
}

#[test]
fn wire_nodes_emits_ruby_kind_from_stdin() {
    let (status, stdout, stderr) = run(&["wire", "nodes"], Some("｜青梅《おうめ》\n"));
    assert!(status.success(), "wire nodes failed: {stderr:?}");
    assert!(stdout.starts_with(ENVELOPE_PREFIX), "envelope: {stdout:?}");
    assert!(stdout.contains(r#""kind":"ruby""#), "nodes: {stdout:?}");
}

#[test]
fn wire_pairs_emits_ruby_pair_with_open_and_close() {
    let (status, stdout, _) = run(&["wire", "pairs"], Some("｜青梅《おうめ》\n"));
    assert!(status.success());
    assert!(stdout.starts_with(ENVELOPE_PREFIX), "envelope: {stdout:?}");
    assert!(stdout.contains(r#""kind":"ruby""#), "pairs: {stdout:?}");
    assert!(
        stdout.contains(r#""open":"#) && stdout.contains(r#""close":"#),
        "pair spans: {stdout:?}"
    );
}

#[test]
fn wire_container_pairs_emits_pair_offsets() {
    let src = "［＃ここから２字下げ］\n本文\n［＃ここで字下げ終わり］\n";
    let (status, stdout, stderr) = run(&["wire", "container-pairs"], Some(src));
    assert!(status.success(), "container-pairs failed: {stderr:?}");
    assert!(stdout.starts_with(ENVELOPE_PREFIX), "envelope: {stdout:?}");
    assert!(
        stdout.contains(r#""offset":"#),
        "expected a container pair: {stdout:?}"
    );
}

#[test]
fn wire_diagnostics_emits_data_for_pua_collision() {
    // `wire diagnostics` is a pure projection: it exits 0 regardless of
    // findings (unlike `check --strict`), and the PUA sentinel produces
    // a `source_contains_pua` entry.
    let (status, stdout, _) = run(&["wire", "diagnostics"], Some("abc\u{E001}def"));
    assert!(status.success(), "diagnostics projection always exits 0");
    assert!(stdout.starts_with(ENVELOPE_PREFIX), "envelope: {stdout:?}");
    assert!(
        stdout.contains(r#""kind":"source_contains_pua""#),
        "diag: {stdout:?}"
    );
}

#[test]
fn wire_gaiji_resolves_reference() {
    let (status, stdout, stderr) = run(&["wire", "gaiji"], Some("※［＃「々」］"));
    assert!(status.success(), "gaiji failed: {stderr:?}");
    assert!(stdout.starts_with(ENVELOPE_PREFIX), "envelope: {stdout:?}");
    assert!(stdout.contains(r#""resolved":"々""#), "gaiji: {stdout:?}");
}

#[test]
fn wire_gaiji_resolutions_is_an_accepted_alias() {
    // The wire-function name `gaiji-resolutions` is an alias for the
    // short, user-facing `gaiji`.
    let (status, stdout, _) = run(&["wire", "gaiji-resolutions"], Some("※［＃「々」］"));
    assert!(status.success());
    assert!(stdout.contains(r#""resolved":"々""#), "alias: {stdout:?}");
}

#[test]
fn wire_slugs_needs_no_input() {
    // `slugs` is a static catalogue: it must succeed with neither stdin
    // nor a file argument.
    let (status, stdout, stderr) = run(&["wire", "slugs"], None);
    assert!(status.success(), "slugs failed: {stderr:?}");
    assert!(stdout.starts_with(ENVELOPE_PREFIX), "envelope: {stdout:?}");
    assert!(stdout.contains(r#""canonical":"#), "slugs: {stdout:?}");
}

#[test]
fn wire_reads_from_a_file_path() {
    let f = write_temp("｜青梅《おうめ》\n");
    let (status, stdout, stderr) = run(&["wire", "nodes", f.path().to_str().unwrap()], None);
    assert!(status.success(), "wire from file failed: {stderr:?}");
    assert!(stdout.contains(r#""kind":"ruby""#), "file path: {stdout:?}");
}

#[test]
fn wire_rejects_unknown_kind() {
    let (status, _, stderr) = run(&["wire", "bogus"], None);
    assert!(!status.success(), "unknown kind must fail");
    assert!(
        stderr.contains("invalid value") || stderr.contains("possible values"),
        "expected clap value error: {stderr:?}"
    );
}
