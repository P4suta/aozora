//! Integration tests for CLI configuration discovery and precedence.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_aozora");

const ONE_PUA: &[u8] = b"a\xee\x80\x81b";

fn run_in(dir: &Path, args: &[&str], envs: &[(&str, &str)], stdin: &[u8]) -> (Option<i32>, String) {
    let empty_xdg = TempDir::new().expect("empty xdg tempdir");
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .current_dir(dir)
        .env_remove("AOZORA_STRICT")
        .env_remove("AOZORA_ENCODING")
        .env_remove("AOZORA_FORMAT")
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR")
        .env_remove("CLICOLOR_FORCE")
        .env_remove("FORCE_COLOR")
        .env("XDG_CONFIG_HOME", empty_xdg.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().expect("spawn aozora");
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

fn write_global_config(xdg: &Path, body: &str) {
    let dir = xdg.join("aozora");
    fs::create_dir_all(&dir).expect("create xdg aozora dir");
    fs::write(dir.join("config.toml"), body).expect("write global config.toml");
}

fn xdg_env(xdg: &TempDir) -> (&'static str, String) {
    (
        "XDG_CONFIG_HOME",
        xdg.path().to_str().expect("utf8 xdg").to_owned(),
    )
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
fn config_format_short_shapes_stderr() {
    let dir = TempDir::new().expect("tempdir");
    write_config(dir.path(), "format = \"short\"\n");
    let (_, stderr) = run_in(dir.path(), &["check"], &[], ONE_PUA);
    assert!(
        stderr.contains("warning[aozora::lex::source_contains_pua]:"),
        "config format=short → rustc-style line: {stderr:?}"
    );
}

#[test]
fn flag_beats_config_format() {
    let dir = TempDir::new().expect("tempdir");
    write_config(dir.path(), "format = \"json\"\n");
    let (_, stderr) = run_in(dir.path(), &["check", "--format", "short"], &[], ONE_PUA);
    assert!(
        stderr.contains("warning[aozora::lex::source_contains_pua]:"),
        "explicit --format short beats config json: {stderr:?}"
    );
}

#[test]
fn config_encoding_utf8_rejects_sjis_bytes() {
    let dir = TempDir::new().expect("tempdir");
    write_config(dir.path(), "encoding = \"utf8\"\n");
    let (code, stderr) = run_in(dir.path(), &["check"], &[], b"\x82\xa0");
    assert_eq!(code, Some(2), "invalid input encoding: {stderr:?}");
}

#[test]
fn unknown_config_key_is_rejected() {
    let dir = TempDir::new().expect("tempdir");
    write_config(dir.path(), "bogus = 1\n");
    let (code, stderr) = run_in(dir.path(), &["check"], &[], ONE_PUA);
    assert_eq!(code, Some(2), "unknown key is a configuration error");
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

fn has_ansi(stderr: &str) -> bool {
    stderr.contains('\u{1b}')
}

fn check_human_stderr(dir: &Path, envs: &[(&str, &str)]) -> String {
    let (_, stderr) = run_in(dir, &["check", "--format", "human"], envs, ONE_PUA);
    stderr
}

#[test]
fn config_color_always_colourises_a_piped_stderr() {
    let dir = TempDir::new().expect("tempdir");
    write_config(dir.path(), "color = \"always\"\n");
    let stderr = check_human_stderr(dir.path(), &[]);
    assert!(
        has_ansi(&stderr),
        "config color=always must colourise even a piped stderr: {stderr:?}"
    );
}

#[test]
fn config_color_never_suppresses_colour() {
    let dir = TempDir::new().expect("tempdir");
    write_config(dir.path(), "color = \"never\"\n");
    let stderr = check_human_stderr(dir.path(), &[("CLICOLOR_FORCE", "1")]);
    assert!(
        !has_ansi(&stderr),
        "config color=never must suppress colour: {stderr:?}"
    );
}

#[test]
fn flag_beats_config_color() {
    let dir = TempDir::new().expect("tempdir");
    write_config(dir.path(), "color = \"never\"\n");
    let (_, stderr) = run_in(
        dir.path(),
        &["check", "--format", "human", "--color", "always"],
        &[],
        ONE_PUA,
    );
    assert!(
        has_ansi(&stderr),
        "explicit --color always beats config never: {stderr:?}"
    );
}

#[test]
fn project_color_beats_the_global_layer() {
    let cwd = TempDir::new().expect("cwd tempdir");
    let xdg = TempDir::new().expect("xdg tempdir");
    write_global_config(xdg.path(), "color = \"never\"\n");
    write_config(cwd.path(), "color = \"always\"\n");
    let env = xdg_env(&xdg);
    let stderr = check_human_stderr(cwd.path(), &[(env.0, env.1.as_str())]);
    assert!(
        has_ansi(&stderr),
        "the project .aozora.toml colour beats the global one: {stderr:?}"
    );
}

#[test]
fn invalid_color_value_is_rejected() {
    let dir = TempDir::new().expect("tempdir");
    write_config(dir.path(), "color = \"rainbow\"\n");
    let (code, stderr) = run_in(dir.path(), &["check"], &[], ONE_PUA);
    assert_eq!(code, Some(2), "unknown color value is a config error");
    assert!(
        stderr.contains("invalid config"),
        "error explains the bad color value: {stderr:?}"
    );
}

#[test]
fn global_config_applies_beneath_the_default() {
    let cwd = TempDir::new().expect("cwd tempdir");
    let xdg = TempDir::new().expect("xdg tempdir");
    write_global_config(xdg.path(), "strict = true\n");
    let env = xdg_env(&xdg);
    let (code, _) = run_in(cwd.path(), &["check"], &[(env.0, env.1.as_str())], ONE_PUA);
    assert_eq!(
        code,
        Some(1),
        "global strict=true (global > default) → exit 1"
    );
}

#[test]
fn project_overrides_global_per_field() {
    let cwd = TempDir::new().expect("cwd tempdir");
    let xdg = TempDir::new().expect("xdg tempdir");
    write_global_config(xdg.path(), "strict = true\nformat = \"json\"\n");
    write_config(cwd.path(), "format = \"short\"\n");
    let env = xdg_env(&xdg);
    let (code, stderr) = run_in(cwd.path(), &["check"], &[(env.0, env.1.as_str())], ONE_PUA);
    assert!(
        stderr.contains("warning[aozora::lex::source_contains_pua]:"),
        "project format=short overrides global json: {stderr:?}"
    );
    assert_eq!(code, Some(1), "global strict survives the per-field merge");
}

#[test]
fn env_overrides_both_project_and_global() {
    let cwd = TempDir::new().expect("cwd tempdir");
    let xdg = TempDir::new().expect("xdg tempdir");
    write_global_config(xdg.path(), "strict = false\n");
    write_config(cwd.path(), "strict = false\n");
    let env = xdg_env(&xdg);
    let (code, _) = run_in(
        cwd.path(),
        &["check"],
        &[(env.0, env.1.as_str()), ("AOZORA_STRICT", "true")],
        ONE_PUA,
    );
    assert_eq!(
        code,
        Some(1),
        "AOZORA_STRICT=true beats project+global strict=false"
    );
}

#[test]
fn flag_overrides_both_project_and_global() {
    let cwd = TempDir::new().expect("cwd tempdir");
    let xdg = TempDir::new().expect("xdg tempdir");
    write_global_config(xdg.path(), "format = \"json\"\n");
    write_config(cwd.path(), "format = \"json\"\n");
    let env = xdg_env(&xdg);
    let (_, stderr) = run_in(
        cwd.path(),
        &["check", "--format", "short"],
        &[(env.0, env.1.as_str())],
        ONE_PUA,
    );
    assert!(
        stderr.contains("warning[aozora::lex::source_contains_pua]:"),
        "explicit --format short beats project+global json: {stderr:?}"
    );
}

#[test]
fn explicit_config_bypasses_project_and_global() {
    let cwd = TempDir::new().expect("cwd tempdir");
    let xdg = TempDir::new().expect("xdg tempdir");
    write_global_config(xdg.path(), "strict = true\n");
    write_config(cwd.path(), "strict = true\n");
    let explicit = cwd.path().join("explicit.toml");
    fs::write(&explicit, "encoding = \"utf8\"\n").expect("write explicit config");
    let env = xdg_env(&xdg);
    let (code, _) = run_in(
        cwd.path(),
        &["check", "--config", explicit.to_str().expect("utf8 path")],
        &[(env.0, env.1.as_str())],
        ONE_PUA,
    );
    assert_eq!(
        code,
        Some(0),
        "--config is a single-file escape hatch: neither layer's strict applies"
    );
}
