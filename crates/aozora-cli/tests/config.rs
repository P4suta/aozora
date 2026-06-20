//! Integration coverage for `.aozora.toml` (src/config.rs).
//!
//! Runs the real binary with its working directory set to a tempdir
//! holding an `.aozora.toml`, so discovery (upward search) and the
//! flag > env > config > default precedence are exercised end to end.
//! `AOZORA_*` vars are cleared per run so the host environment cannot
//! perturb the assertions.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_aozora");

// One PUA sentinel (U+E001) → a single `source_contains_pua` warning.
const ONE_PUA: &[u8] = b"a\xee\x80\x81b";

/// Run `aozora <args>` in `dir` with `envs` set, feeding `stdin`.
/// Returns `(exit_code, stderr)`.
fn run_in(dir: &Path, args: &[&str], envs: &[(&str, &str)], stdin: &[u8]) -> (Option<i32>, String) {
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .current_dir(dir)
        .env_remove("AOZORA_STRICT")
        .env_remove("AOZORA_ENCODING")
        .env_remove("AOZORA_DIAGNOSTIC_FORMAT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().expect("spawn aozora");
    // The child may exit before reading stdin (a config error surfaces
    // first), closing the pipe — tolerate the resulting broken pipe.
    let _drop = child.stdin.as_mut().expect("piped stdin").write_all(stdin);
    let output = child.wait_with_output().expect("wait for aozora");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn write_config(dir: &Path, body: &str) {
    fs::write(dir.join(".aozora.toml"), body).expect("write .aozora.toml");
}

#[test]
fn default_tolerates_diagnostics() {
    let dir = TempDir::new().expect("tempdir");
    let (code, _) = run_in(dir.path(), &["check"], &[], ONE_PUA);
    assert_eq!(code, Some(0), "no config → diagnostics tolerated, exit 0");
}

#[test]
fn config_strict_makes_diagnostics_fail() {
    let dir = TempDir::new().expect("tempdir");
    write_config(dir.path(), "strict = true\n");
    let (code, _) = run_in(dir.path(), &["check"], &[], ONE_PUA);
    assert_eq!(code, Some(1), "config strict=true → exit 1 on a diagnostic");
}

#[test]
fn env_strict_overrides_absent_config() {
    let dir = TempDir::new().expect("tempdir");
    let (code, _) = run_in(
        dir.path(),
        &["check"],
        &[("AOZORA_STRICT", "true")],
        ONE_PUA,
    );
    assert_eq!(code, Some(1), "AOZORA_STRICT=true (env > default) → exit 1");
}

#[test]
fn config_diagnostic_format_short_shapes_stderr() {
    let dir = TempDir::new().expect("tempdir");
    write_config(dir.path(), "diagnostic-format = \"short\"\n");
    let (_, stderr) = run_in(dir.path(), &["check"], &[], ONE_PUA);
    assert!(
        stderr.contains("warning[aozora::lex::source_contains_pua]:"),
        "config diagnostic-format=short → rustc-style line: {stderr:?}"
    );
}

#[test]
fn flag_beats_config_diagnostic_format() {
    let dir = TempDir::new().expect("tempdir");
    write_config(dir.path(), "diagnostic-format = \"json\"\n");
    let (_, stderr) = run_in(
        dir.path(),
        &["check", "--diagnostic-format", "short"],
        &[],
        ONE_PUA,
    );
    assert!(
        stderr.contains("warning[aozora::lex::source_contains_pua]:"),
        "explicit --diagnostic-format short beats config json: {stderr:?}"
    );
}

#[test]
fn config_encoding_utf8_rejects_sjis_bytes() {
    let dir = TempDir::new().expect("tempdir");
    write_config(dir.path(), "encoding = \"utf8\"\n");
    // 「あ」in Shift_JIS (0x82 0xA0) is not valid UTF-8.
    let (code, stderr) = run_in(dir.path(), &["check"], &[], b"\x82\xa0");
    assert_ne!(
        code,
        Some(0),
        "utf8 config rejects non-UTF-8 input: {stderr:?}"
    );
}

#[test]
fn unknown_config_key_is_rejected() {
    let dir = TempDir::new().expect("tempdir");
    write_config(dir.path(), "bogus = 1\n");
    let (code, stderr) = run_in(dir.path(), &["check"], &[], ONE_PUA);
    assert_ne!(code, Some(0), "unknown key → non-zero exit: {stderr:?}");
    assert!(
        stderr.contains("invalid config"),
        "error explains the bad config: {stderr:?}"
    );
}

#[test]
fn explicit_config_path_is_used() {
    let dir = TempDir::new().expect("tempdir");
    let cfg = dir.path().join("custom.toml");
    fs::write(&cfg, "strict = true\n").expect("write custom config");
    let (code, _) = run_in(
        dir.path(),
        &["check", "--config", cfg.to_str().expect("utf8 path")],
        &[],
        ONE_PUA,
    );
    assert_eq!(code, Some(1), "--config PATH applied → strict exit 1");
}
