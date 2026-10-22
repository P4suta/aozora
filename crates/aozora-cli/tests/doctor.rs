//! End-to-end tests for `aozora doctor`.

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

mod common;

fn doctor_command(work: &Path, path_dir: &Path) -> Command {
    let mut cmd = common::hermetic_command();
    cmd.arg("doctor")
        .current_dir(work)
        .env("PATH", path_dir)
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR")
        .env_remove("CLICOLOR_FORCE")
        .env_remove("AOZORA_ENCODING")
        .env_remove("AOZORA_FORMAT")
        .env_remove("AOZORA_STRICT");
    cmd
}

fn run_doctor() -> (i32, String) {
    let work = TempDir::new().expect("work tempdir");
    let path_dir = TempDir::new().expect("empty PATH tempdir");
    let output = doctor_command(work.path(), path_dir.path())
        .output()
        .expect("spawn aozora doctor");
    (
        output.status.code().expect("doctor exits normally"),
        String::from_utf8(output.stdout).expect("doctor stdout is UTF-8"),
    )
}

fn doctor_filters() -> Vec<(&'static str, &'static str)> {
    vec![(r"searched up from [^)]*", "searched up from [CWD]")]
}

#[test]
fn doctor_all_green_snapshot() {
    let (code, stdout) = run_doctor();
    assert_eq!(code, 0, "an all-green doctor exits 0:\n{stdout}");
    insta::with_settings!({ filters => doctor_filters() }, {
        insta::assert_snapshot!(stdout);
    });
}

#[test]
fn doctor_reports_missing_optional_tools_without_blocking() {
    let (code, stdout) = run_doctor();
    assert_eq!(
        code, 0,
        "missing optional tools are not blocking:\n{stdout}"
    );
    assert!(
        stdout.contains("pandoc       not found on PATH"),
        "pandoc probe reported: {stdout:?}"
    );
    assert!(
        stdout.contains("All checks passed."),
        "all-green summary: {stdout:?}"
    );
}

#[test]
fn doctor_reports_effective_settings_with_default_sources() {
    let (_, stdout) = run_doctor();
    assert!(stdout.contains("encoding   auto     default"), "{stdout:?}");
    assert!(stdout.contains("format     auto     default"), "{stdout:?}");
    assert!(stdout.contains("strict     false    default"), "{stdout:?}");
    assert!(stdout.contains("color      auto     default"), "{stdout:?}");
    assert!(
        stdout.contains("lang       en       env AOZORA_LANG"),
        "{stdout:?}"
    );
}

#[test]
fn doctor_attributes_project_config_as_the_source() {
    let work = TempDir::new().expect("work tempdir");
    let path_dir = TempDir::new().expect("empty PATH tempdir");
    fs::write(
        work.path().join(".aozora.toml"),
        "strict = true\nencoding = \"sjis\"\n",
    )
    .expect("write project config");

    let output = doctor_command(work.path(), path_dir.path())
        .output()
        .expect("spawn aozora doctor");
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");

    assert_eq!(
        output.status.code(),
        Some(0),
        "a valid config is not blocking"
    );
    assert!(
        stdout.contains("configuration parsed cleanly (no unknown keys)"),
        "clean-parse line: {stdout:?}"
    );
    assert!(stdout.contains("strict     true     project"), "{stdout:?}");
    assert!(stdout.contains("encoding   sjis     project"), "{stdout:?}");
    assert!(stdout.contains("format     auto     default"), "{stdout:?}");
}

#[test]
fn doctor_flags_an_unknown_config_key_as_blocking() {
    let work = TempDir::new().expect("work tempdir");
    let path_dir = TempDir::new().expect("empty PATH tempdir");
    fs::write(work.path().join(".aozora.toml"), "colour = \"never\"\n")
        .expect("write project config");

    let output = doctor_command(work.path(), path_dir.path())
        .output()
        .expect("spawn aozora doctor");
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");

    assert_eq!(
        output.status.code(),
        Some(1),
        "a malformed config is a blocking failure:\n{stdout}"
    );
    assert!(
        stdout.contains("configuration error:"),
        "the error is surfaced under Configuration: {stdout:?}"
    );
    assert!(
        stdout.contains("unknown field `colour`"),
        "the deny_unknown_fields message names the bad key: {stdout:?}"
    );
    assert!(
        stdout.contains("`encoding`, `format`, `strict`, `color`, `lang`"),
        "and the valid set: {stdout:?}"
    );
    assert!(
        stdout.contains("1 problem(s) found."),
        "the summary counts it: {stdout:?}"
    );
}

#[test]
fn doctor_localizes_the_report_under_lang() {
    let work = TempDir::new().expect("work tempdir");
    let path_dir = TempDir::new().expect("empty PATH tempdir");
    let output = doctor_command(work.path(), path_dir.path())
        .arg("--lang")
        .arg("ja")
        .output()
        .expect("spawn aozora doctor");
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");

    assert_eq!(output.status.code(), Some(0));
    assert!(
        stdout.contains("実効設定"),
        "ja settings heading: {stdout:?}"
    );
    assert!(
        stdout.contains("外部ツール"),
        "ja tools heading: {stdout:?}"
    );
    assert!(
        stdout.contains("encoding   auto     default"),
        "literal row: {stdout:?}"
    );
    assert!(
        stdout.contains("lang       ja       flag"),
        "lang from flag: {stdout:?}"
    );
}

#[test]
fn doctor_strict_matches_the_or_runtime_when_config_forces_it() {
    let work = TempDir::new().expect("work tempdir");
    let path_dir = TempDir::new().expect("empty PATH tempdir");
    fs::write(work.path().join(".aozora.toml"), "strict = true\n").expect("write project config");

    let output = doctor_command(work.path(), path_dir.path())
        .env("AOZORA_STRICT", "false")
        .output()
        .expect("spawn aozora doctor");
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");

    assert_eq!(
        output.status.code(),
        Some(0),
        "a valid config is not blocking:\n{stdout}"
    );
    assert!(
        stdout.contains("strict     true     project"),
        "config forces strict ON, sourced to project despite AOZORA_STRICT=false: {stdout:?}"
    );
    assert!(
        !stdout.contains("strict     false"),
        "doctor must not report the opposite (false) effective value: {stdout:?}"
    );

    let diag = work.path().join("diag.txt");
    fs::write(&diag, "［＃ここから\n").expect("write diag fixture");
    let check = common::hermetic_command()
        .arg("check")
        .arg(&diag)
        .current_dir(work.path())
        .env("AOZORA_STRICT", "false")
        .output()
        .expect("spawn aozora check");
    assert_eq!(
        check.status.code(),
        Some(1),
        "check runs strict (exit 1) under the same config + env doctor reported"
    );
}

#[test]
fn doctor_flags_env_values_the_runtime_would_reject() {
    let work = TempDir::new().expect("work tempdir");
    let path_dir = TempDir::new().expect("empty PATH tempdir");

    let output = doctor_command(work.path(), path_dir.path())
        .env("AOZORA_ENCODING", "SJIS")
        .env("AOZORA_FORMAT", "JSON")
        .env("AOZORA_STRICT", "on")
        .output()
        .expect("spawn aozora doctor");
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");

    assert_eq!(
        output.status.code(),
        Some(1),
        "invalid env values are blocking:\n{stdout}"
    );
    assert!(
        stdout
            .contains("AOZORA_ENCODING=SJIS is set but not a valid value; aozora would reject it"),
        "encoding rejection surfaced: {stdout:?}"
    );
    assert!(
        stdout.contains("AOZORA_FORMAT=JSON is set but not a valid value; aozora would reject it"),
        "format rejection surfaced: {stdout:?}"
    );
    assert!(
        stdout.contains("AOZORA_STRICT=on is set but not a valid value; aozora would reject it"),
        "strict rejection surfaced: {stdout:?}"
    );
    assert!(
        !stdout.contains("encoding   sjis"),
        "no clean sjis row: {stdout:?}"
    );
    assert!(
        !stdout.contains("format     json"),
        "no clean json row: {stdout:?}"
    );
    assert!(
        stdout.contains("3 problem(s) found."),
        "the summary counts all three: {stdout:?}"
    );

    let ok = work.path().join("ok.txt");
    fs::write(&ok, "hello\n").expect("write input");
    let check = common::hermetic_command()
        .arg("check")
        .arg(&ok)
        .current_dir(work.path())
        .env("AOZORA_ENCODING", "SJIS")
        .output()
        .expect("spawn aozora check");
    assert_eq!(
        check.status.code(),
        Some(2),
        "the runtime rejects AOZORA_ENCODING=SJIS with a usage error (exit 2)"
    );
}
