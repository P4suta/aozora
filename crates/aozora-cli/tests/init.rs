//! End-to-end tests for `aozora init` — the project scaffolder.
//!
//! Every spawn goes through [`common::hermetic_command`] (which pins
//! `AOZORA_LANG=en` and seals the global XDG config layer) and runs in a fresh
//! `TempDir`, so the report and the written files are a pure function of the
//! flags, not of the host environment.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

mod common;

/// Run `aozora init <args…>` inside `dir`, returning the raw [`Output`].
fn init_in(dir: &Path, args: &[&str]) -> Output {
    common::hermetic_command()
        .arg("init")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("spawn aozora init")
}

#[test]
fn init_scaffold_snapshot() {
    let work = TempDir::new().expect("work tempdir");
    let output = init_in(work.path(), &[]);
    assert_eq!(output.status.code(), Some(0), "a fresh scaffold exits 0");
    let stdout = String::from_utf8(output.stdout).expect("init stdout is UTF-8");
    insta::assert_snapshot!(stdout);
}

#[test]
fn init_writes_the_three_files_with_expected_content() {
    let work = TempDir::new().expect("work tempdir");
    let output = init_in(work.path(), &[]);
    assert_eq!(output.status.code(), Some(0));

    // The commented config template documents every ConfigFile key.
    let config = fs::read_to_string(work.path().join(".aozora.toml")).expect("config written");
    for key in ["encoding", "format", "strict", "color", "lang"] {
        assert!(
            config.contains(key),
            "config template mentions `{key}`: {config}"
        );
    }
    // The sample exercises the three advertised notations.
    let sample = fs::read_to_string(work.path().join("hon.aozora")).expect("sample written");
    assert!(sample.contains("《おうめ》"), "ruby reading: {sample}");
    assert!(sample.contains("に傍点"), "傍点 directive: {sample}");
    assert!(sample.contains("字下げ"), "字下げ directive: {sample}");
    // The .gitignore is there too.
    let ignore = fs::read_to_string(work.path().join(".gitignore")).expect("gitignore written");
    assert!(ignore.contains("*.html"), "ignores rendered HTML: {ignore}");
}

#[test]
fn init_scaffolds_a_named_directory_creating_it() {
    let work = TempDir::new().expect("work tempdir");
    let output = init_in(work.path(), &["myproject"]);
    assert_eq!(output.status.code(), Some(0), "creates the named dir");
    assert!(
        work.path().join("myproject").join(".aozora.toml").is_file(),
        "the config landed under the named directory",
    );
    assert!(
        work.path().join("myproject").join("hon.aozora").is_file(),
        "and so did the sample",
    );
}

#[test]
fn init_is_idempotent_and_never_clobbers_without_force() {
    let work = TempDir::new().expect("work tempdir");
    // First run creates everything.
    assert_eq!(init_in(work.path(), &[]).status.code(), Some(0));
    // Hand-edit the config so we can prove the second run leaves it untouched.
    let config = work.path().join(".aozora.toml");
    fs::write(&config, "strict = true\n").expect("edit config");

    let second = init_in(work.path(), &[]);
    assert_eq!(second.status.code(), Some(0), "a re-run still succeeds");
    let stdout = String::from_utf8(second.stdout).expect("stdout is UTF-8");
    assert!(
        stdout.contains(".aozora.toml     skipped (already exists; use --force to overwrite)"),
        "the existing file is reported skipped: {stdout}",
    );
    assert_eq!(
        fs::read_to_string(&config).expect("read config"),
        "strict = true\n",
        "the user's edit survives — no silent clobber",
    );
}

#[test]
fn init_force_overwrites_existing_files() {
    let work = TempDir::new().expect("work tempdir");
    let config = work.path().join(".aozora.toml");
    fs::write(&config, "strict = true\n").expect("seed a stale config");

    let output = init_in(work.path(), &["--force"]);
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert!(
        stdout.contains(".aozora.toml     overwritten"),
        "--force overwrites and says so: {stdout}",
    );
    let rewritten = fs::read_to_string(&config).expect("read config");
    assert!(
        rewritten.contains("# encoding = \"auto\""),
        "the template replaced the stale content: {rewritten}",
    );
}

#[test]
fn init_no_sample_and_no_gitignore_opt_out() {
    let work = TempDir::new().expect("work tempdir");
    let output = init_in(work.path(), &["--no-sample", "--no-gitignore"]);
    assert_eq!(output.status.code(), Some(0));
    assert!(
        work.path().join(".aozora.toml").is_file(),
        "config still written"
    );
    assert!(
        !work.path().join("hon.aozora").exists(),
        "--no-sample suppresses the sample",
    );
    assert!(
        !work.path().join(".gitignore").exists(),
        "--no-gitignore suppresses the ignore file",
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    assert!(
        !stdout.contains("hon.aozora"),
        "the next-steps drop the missing-sample commands: {stdout}",
    );
}

#[test]
fn init_localizes_the_report_under_lang() {
    let work = TempDir::new().expect("work tempdir");
    let output = common::hermetic_command()
        .arg("init")
        .arg("--lang")
        .arg("ja")
        .current_dir(work.path())
        .output()
        .expect("spawn aozora init");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).expect("stdout is UTF-8");
    // Prose axis follows --lang...
    assert!(stdout.contains("作成"), "ja created word: {stdout}");
    assert!(stdout.contains("次のステップ"), "ja next-steps: {stdout}");
    // ...while the file names stay literal.
    assert!(
        stdout.contains(".aozora.toml"),
        "literal file name: {stdout}"
    );
}

#[test]
fn init_scaffold_renders_and_checks_cleanly() {
    // The whole point: right after `init`, the sample renders and checks with
    // no diagnostics — drive the real binary against the scaffolded files.
    let work = TempDir::new().expect("work tempdir");
    assert_eq!(init_in(work.path(), &[]).status.code(), Some(0));
    let sample = work.path().join("hon.aozora");

    let check = Command::new(env!("CARGO_BIN_EXE_aozora"))
        .arg("check")
        .arg(&sample)
        .env("AOZORA_LANG", "en")
        .output()
        .expect("spawn aozora check");
    assert_eq!(
        check.status.code(),
        Some(0),
        "the scaffolded sample checks clean: {}",
        String::from_utf8_lossy(&check.stderr),
    );

    let render = Command::new(env!("CARGO_BIN_EXE_aozora"))
        .arg("render")
        .arg(&sample)
        .output()
        .expect("spawn aozora render");
    assert_eq!(render.status.code(), Some(0));
    let html = String::from_utf8(render.stdout).expect("render stdout is UTF-8");
    assert!(html.contains("<ruby>"), "ruby reached HTML: {html}");
    assert!(html.contains("aozora-bouten"), "傍点 reached HTML: {html}");
    assert!(
        html.contains("aozora-container-indent"),
        "字下げ reached HTML: {html}",
    );
}
