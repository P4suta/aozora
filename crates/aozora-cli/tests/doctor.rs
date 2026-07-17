//! End-to-end tests for `aozora doctor` — the end-user runtime self-check.
//!
//! Every spawn is pinned hermetic: [`common::hermetic_command`] seals the
//! global XDG config layer and the message language (`AOZORA_LANG=en`), and
//! each test additionally runs in a fresh empty working directory with an empty
//! `PATH` (so `pandoc` deterministically reports "not found") and
//! with the colour env vars stripped — so the report is a pure function of the
//! fixture, not of the host box `doctor` is meant to inspect.

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

mod common;

/// A doctor command sealed for a deterministic report: the shared hermetic
/// environment, plus a fresh working directory (`work`), an empty `PATH`
/// (`path_dir`), stripped `NO_COLOR` / `CLICOLOR` / `CLICOLOR_FORCE`, and
/// stripped `AOZORA_ENCODING` / `AOZORA_FORMAT` / `AOZORA_STRICT` so the
/// effective-settings dump reflects the fixture, not the host's shell — a test
/// that exercises one of those sets it back explicitly.
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

/// Run `aozora doctor` in a fresh empty dir with an empty PATH, returning
/// `(exit_code, stdout)`.
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

/// Filter the one non-deterministic fragment — the working directory echoed in
/// the "searched up from …" line — so the golden captures the surface, not the
/// tempdir path.
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
    // pandoc is optional, so its absence is advisory only — the report still
    // exits 0 and closes with the all-passed summary. (The LSP is built into
    // the binary, so there is no external daemon to probe.)
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
    // No config, no encoding/format env: each layered setting is the default.
    assert!(stdout.contains("encoding   auto     default"), "{stdout:?}");
    assert!(stdout.contains("format     auto     default"), "{stdout:?}");
    assert!(stdout.contains("strict     false    default"), "{stdout:?}");
    assert!(stdout.contains("color      auto     default"), "{stdout:?}");
    // The hermetic env pins AOZORA_LANG=en, so lang is attributed to that env.
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
    // The two project-set keys are attributed to the project layer; the rest
    // stay defaults — the provenance is the point of the settings dump.
    assert!(stdout.contains("strict     true     project"), "{stdout:?}");
    assert!(stdout.contains("encoding   sjis     project"), "{stdout:?}");
    assert!(stdout.contains("format     auto     default"), "{stdout:?}");
}

#[test]
fn doctor_flags_an_unknown_config_key_as_blocking() {
    let work = TempDir::new().expect("work tempdir");
    let path_dir = TempDir::new().expect("empty PATH tempdir");
    // `colour` is a plausible misspelling of the real `color` key; the
    // deny_unknown_fields loader must reject it and doctor must surface that
    // actionably (naming the valid set) and exit 1.
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
    // Prose axis follows --lang: Japanese headings...
    assert!(
        stdout.contains("実効設定"),
        "ja settings heading: {stdout:?}"
    );
    assert!(
        stdout.contains("外部ツール"),
        "ja tools heading: {stdout:?}"
    );
    // ...while the setting identifiers, tags, and source labels stay literal.
    assert!(
        stdout.contains("encoding   auto     default"),
        "literal row: {stdout:?}"
    );
    // And --lang is attributed to the flag.
    assert!(
        stdout.contains("lang       ja       flag"),
        "lang from flag: {stdout:?}"
    );
}

#[test]
fn doctor_strict_matches_the_or_runtime_when_config_forces_it() {
    // BUG 1: AOZORA_STRICT=false + config strict=true. The runtime computes
    // `args.strict || cfg.strict.unwrap_or(false)` = `false || true` = strict
    // ON. Doctor must report the same effective value, sourced to the config
    // layer that forced it — not the opposite (`false`, sourced to the env), the
    // way the old env-over-config layering did.
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

    // Parity: `aozora check` under the identical config + env runs strict, so a
    // tolerated diagnostic becomes exit 1 — doctor's report and the runtime agree.
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
    // BUG 2: the runtime's clap parser is case-sensitive for --encoding /
    // --format and accepts only the literal true / false for --strict. Doctor
    // must flag AOZORA_ENCODING=SJIS / AOZORA_FORMAT=JSON / AOZORA_STRICT=on as
    // blocking problems (exit 1), not report them as clean effective settings.
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
    // None are shown as clean effective rows (the divergence the old case-
    // insensitive / boolish parsing produced).
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

    // Parity: `aozora check` under the same env is a hard clap rejection (exit
    // 2) — doctor's exit-1 warning is the actionable heads-up for exactly that.
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
