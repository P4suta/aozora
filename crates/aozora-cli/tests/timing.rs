//! Integration tests for timing reports.

use std::io::Write;
use std::process::Stdio;

use aozora::json::SCHEMA_VERSION;

mod common;

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

const RUBY: &[u8] = "｜青《あ》\n".as_bytes();

#[test]
fn timing_auto_selects_json_when_stderr_is_piped() {
    let (_, stderr) = run(&["render", "--timing"], RUBY);
    let value: serde_json::Value =
        serde_json::from_str(stderr.trim()).expect("auto → json envelope parses");
    assert_eq!(
        value["schemaVersion"], SCHEMA_VERSION,
        "auto resolves to the json envelope off a TTY: {stderr:?}"
    );
}

#[test]
fn timing_leaves_stdout_untouched() {
    let (with, _) = run(&["render", "--timing"], RUBY);
    let (without, _) = run(&["render"], RUBY);
    assert_eq!(
        with, without,
        "--timing must not alter stdout: {with:?} vs {without:?}"
    );
}

#[test]
fn timing_json_envelope_carries_phases_and_total_under_data() {
    let (_, stderr) = run(&["render", "--timing"], RUBY);
    let value: serde_json::Value = serde_json::from_str(stderr.trim()).expect("timing json parses");
    assert_eq!(
        value["schemaVersion"], SCHEMA_VERSION,
        "carries the wire schemaVersion: {stderr:?}"
    );
    let names: Vec<&str> = value["data"]["phases"]
        .as_array()
        .expect("data.phases is an array")
        .iter()
        .map(|p| p["name"].as_str().expect("phase name is a string"))
        .collect();
    assert!(
        names.contains(&"read") && names.contains(&"parse") && names.contains(&"render"),
        "json names read/parse/render phases: {names:?}"
    );
    assert!(
        value["data"]["totalNanos"].as_u64().is_some(),
        "data.totalNanos is a number: {stderr:?}"
    );
}

#[test]
fn timing_json_does_not_pollute_stdout() {
    let (with, _) = run(&["render", "--timing"], RUBY);
    let (without, _) = run(&["render"], RUBY);
    assert_eq!(with, without, "json timing must not alter stdout");
}

#[test]
fn no_timing_flag_writes_nothing_to_stderr() {
    let (_, stderr) = run(&["render"], RUBY);
    assert!(stderr.is_empty(), "no --timing → silent stderr: {stderr:?}");
}
