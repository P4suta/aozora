//! Integration coverage for `.aozora.toml` (src/config.rs).
//!
//! Runs the real binary with its working directory set to a tempdir
//! holding an `.aozora.toml`, so discovery (upward search), the global
//! (XDG) layer, and the flag > env > project > global > default precedence
//! are exercised end to end.
//!
//! Hermeticity: every run clears `AOZORA_*` and pins `XDG_CONFIG_HOME` to a
//! throwaway empty tempdir, so neither the host environment nor the
//! developer's real `~/.config/aozora/` can perturb an assertion. A test
//! that means to exercise the global layer points `XDG_CONFIG_HOME` at its
//! own tempdir (holding `aozora/config.toml`) via `envs`, which — applied
//! last — overrides the empty default.

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
    // Pin the global (XDG) config base to an empty tempdir so the developer's
    // real ~/.config/aozora/ can never perturb the result. A test exercising
    // the global layer overrides XDG_CONFIG_HOME through `envs`, which the
    // loop below applies after this default.
    let empty_xdg = TempDir::new().expect("empty xdg tempdir");
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .current_dir(dir)
        .env_remove("AOZORA_STRICT")
        .env_remove("AOZORA_ENCODING")
        .env_remove("AOZORA_FORMAT")
        .env("XDG_CONFIG_HOME", empty_xdg.path())
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

/// Write a global config at `<xdg>/aozora/config.toml`, creating the
/// `aozora/` subdir. `xdg` is the `XDG_CONFIG_HOME` base the caller then
/// threads through `run_in`'s `envs`.
fn write_global_config(xdg: &Path, body: &str) {
    let dir = xdg.join("aozora");
    fs::create_dir_all(&dir).expect("create xdg aozora dir");
    fs::write(dir.join("config.toml"), body).expect("write global config.toml");
}

/// `("XDG_CONFIG_HOME", <path>)` env pair for a tempdir, spelled once.
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

// --- the `color` key: parsed and validated even while wiring is deferred ---

#[test]
fn color_key_is_accepted() {
    let dir = TempDir::new().expect("tempdir");
    write_config(dir.path(), "color = \"never\"\n");
    // `color` is a known field, so it parses cleanly (deny_unknown_fields
    // does not fire); it must not be rejected the way a mistyped key is.
    let (code, stderr) = run_in(dir.path(), &["check"], &[], ONE_PUA);
    assert_eq!(
        code,
        Some(0),
        "color=never parses, diagnostic tolerated: {stderr:?}"
    );
    assert!(
        !stderr.contains("invalid config"),
        "the known `color` key must not be rejected: {stderr:?}"
    );
}

#[test]
fn invalid_color_value_is_rejected() {
    let dir = TempDir::new().expect("tempdir");
    write_config(dir.path(), "color = \"rainbow\"\n");
    let (code, stderr) = run_in(dir.path(), &["check"], &[], ONE_PUA);
    assert_ne!(
        code,
        Some(0),
        "unknown color value → non-zero exit: {stderr:?}"
    );
    assert!(
        stderr.contains("invalid config"),
        "error explains the bad color value: {stderr:?}"
    );
}

// --- the global (XDG) layer and its composition with the project layer ---

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
    // Global sets BOTH strict and the diagnostic format; the project file
    // overrides only the format, leaving strict to fall through from global.
    write_global_config(xdg.path(), "strict = true\nformat = \"json\"\n");
    write_config(cwd.path(), "format = \"short\"\n");
    let env = xdg_env(&xdg);
    let (code, stderr) = run_in(cwd.path(), &["check"], &[(env.0, env.1.as_str())], ONE_PUA);
    // Project's format wins (short → rustc-style line)...
    assert!(
        stderr.contains("warning[aozora::lex::source_contains_pua]:"),
        "project format=short overrides global json: {stderr:?}"
    );
    // ...while global's strict, which the project left unset, still applies:
    // proof the merge is per-field, not whole-file.
    assert_eq!(code, Some(1), "global strict survives the per-field merge");
}

#[test]
fn env_overrides_both_project_and_global() {
    let cwd = TempDir::new().expect("cwd tempdir");
    let xdg = TempDir::new().expect("xdg tempdir");
    // Neither layer enables strict; the environment does — and wins over both.
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
    // Both discovery layers demand strict; the explicit file does not.
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
    // The explicit file sets no `strict`, and `--config` bypasses BOTH the
    // project .aozora.toml and the global config.toml, so the diagnostic is
    // tolerated (exit 0) rather than promoted to a failure by either layer.
    assert_eq!(
        code,
        Some(0),
        "--config is a single-file escape hatch: neither layer's strict applies"
    );
}
