//! End-to-end smoke tests for the `aozora` binary.

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use aozora::json::SCHEMA_VERSION;

mod common;

struct TempFile {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl TempFile {
    fn path(&self) -> &Path {
        &self.path
    }
}

fn write_temp(contents: &str) -> TempFile {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("aozora-cli-test.txt");
    fs::write(&path, contents).expect("write temp");
    TempFile { _dir: dir, path }
}

fn run(args: &[&str], stdin: Option<&str>) -> (ExitStatus, String, String) {
    let mut cmd = common::hermetic_command();
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    if stdin.is_some() {
        cmd.stdin(Stdio::piped());
    }
    let mut child = cmd.spawn().expect("spawn aozora");
    if let Some(s) = stdin {
        match child
            .stdin
            .as_mut()
            .expect("piped stdin")
            .write_all(s.as_bytes())
        {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {}
            Err(e) => panic!("write stdin: {e}"),
        }
    }
    drop(child.stdin.take());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait().expect("poll aozora") {
            Some(_) => break,
            None if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            None => {
                drop(child.kill());
                drop(child.wait());
                panic!("aozora did not exit: {args:?}");
            }
        }
    }
    let output = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    (output.status, stdout, stderr)
}

#[test]
fn version_subcommand_succeeds() {
    let (status, stdout, _) = run(&["--version"], None);
    assert!(status.success(), "version exited non-zero: {status:?}");
    assert!(
        stdout.contains("aozora"),
        "version output missing crate name: {stdout:?}"
    );
}

#[test]
fn no_args_shows_help_and_exits_non_zero() {
    let (status, _, stderr) = run(&[], None);
    assert!(!status.success(), "expected failure on missing subcommand");
    assert!(
        stderr.contains("Usage:") || stderr.contains("usage:"),
        "expected clap help banner, got: {stderr:?}"
    );
}

#[test]
fn check_clean_input_exits_zero() {
    let f = write_temp("｜青梅《おうめ》\n");
    let (status, _, stderr) = run(&["check", f.path().to_str().unwrap()], None);
    assert!(status.success(), "check failed on clean input: {stderr:?}");
    assert!(
        stderr.is_empty(),
        "no diagnostics → empty stderr: {stderr:?}"
    );
}

#[test]
fn check_clean_input_via_stdin() {
    let (status, _, stderr) = run(&["check"], Some("｜青梅《おうめ》\n"));
    assert!(
        status.success(),
        "stdin clean input must succeed: {stderr:?}"
    );
}

#[test]
fn check_strict_fails_on_pua_collision() {
    let src = "abc\u{E001}def";
    let (status_relaxed, _, stderr_relaxed) = run(&["check"], Some(src));
    assert!(
        status_relaxed.success(),
        "without --strict, check exits 0 even on diagnostics: {stderr_relaxed:?}"
    );
    assert!(
        stderr_relaxed.contains("PUA") || !stderr_relaxed.is_empty(),
        "expected diagnostic on stderr: {stderr_relaxed:?}"
    );

    let (status_strict, _, _) = run(&["check", "--strict"], Some(src));
    assert!(
        !status_strict.success(),
        "with --strict, check must exit non-zero on any diagnostic"
    );
}

#[test]
fn every_document_projection_shares_diagnostic_flags_and_exit_codes() {
    let source = "abc\u{E001}def";
    for args in [
        &["render", "--strict", "--format", "short"][..],
        &["inspect", "nodes", "--strict", "--format", "short"][..],
        &["pandoc", "--strict", "--format", "short"][..],
        &["fmt", "--strict", "--format", "short"][..],
    ] {
        let (status, _, stderr) = run(args, Some(source));
        assert_eq!(
            status.code(),
            Some(1),
            "{args:?} must apply the shared strict exit: {stderr:?}",
        );
        assert!(
            stderr.contains("source_contains_pua"),
            "{args:?} must apply the shared diagnostic format: {stderr:?}",
        );
    }
}

#[test]
fn fmt_default_prints_canonical_form_on_stdout() {
    let (status, stdout, _) = run(&["fmt"], Some("｜日本《にほん》"));
    assert!(status.success(), "fmt should succeed");
    assert!(
        stdout.contains("日本《にほん》") && !stdout.contains('｜'),
        "expected bare canonical form: {stdout:?}"
    );
}

#[test]
fn fmt_check_succeeds_on_already_canonical_input() {
    let canonical = "日本《にほん》\n";
    let (status, stdout, stderr) = run(&["fmt", "--check"], Some(canonical));
    assert!(
        status.success(),
        "canonical input must pass --check: stderr={stderr:?}"
    );
    assert!(
        stdout.is_empty(),
        "--check stays silent on stdout: {stdout:?}"
    );
}

#[test]
fn fmt_json_reports_the_processed_file() {
    let file = write_temp("plain text\n");
    let path = file.path().to_str().expect("utf8 temp path");
    let (status, stdout, stderr) = run(&["fmt", "--json", "--format", "json", path], None);
    assert!(status.success(), "fmt --json failed: {stderr:?}");
    assert!(
        stderr.is_empty(),
        "clean input has no diagnostics: {stderr:?}"
    );
    let report: serde_json::Value = serde_json::from_str(&stdout).expect("fmt report json");
    assert_eq!(report["schemaVersion"], SCHEMA_VERSION);
    assert_eq!(report["data"]["formatted"], true);
    let files = report["data"]["files"].as_array().expect("files array");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["path"], path);
    assert_eq!(files[0]["status"], "ok");
}

#[test]
fn fmt_check_fails_on_non_canonical_input() {
    let non_canonical = "｜日本《にほん》";
    let (status, _, stderr) = run(&["fmt", "--check"], Some(non_canonical));
    assert!(!status.success(), "non-canonical input must fail --check");
    assert!(
        stderr.contains("would be reformatted"),
        "expected diff hint on stderr: {stderr:?}"
    );
}

#[test]
fn fmt_check_and_write_are_mutually_exclusive() {
    let (status, _, stderr) = run(&["fmt", "--check", "--write", "-"], None);
    assert!(
        !status.success(),
        "clap should reject mutually exclusive flags"
    );
    assert!(
        stderr.contains("cannot be used") || stderr.contains("conflicts"),
        "expected clap conflict hint: {stderr:?}"
    );
}

#[test]
fn fmt_write_overwrites_file_on_disk() {
    let f = write_temp("｜日本《にほん》");
    let path = f.path().to_str().unwrap();
    let (status, stdout, stderr) = run(&["fmt", "--write", path], None);
    assert!(status.success(), "fmt --write must succeed: {stderr:?}");
    assert!(stdout.is_empty(), "--write without --list stays silent");
    let written = fs::read_to_string(path).expect("read back");
    assert!(
        written.contains("日本《にほん》") && !written.contains('｜'),
        "file must contain bare canonical output: {written:?}"
    );
}

#[test]
fn fmt_write_preserves_explicit_shift_jis_encoding() {
    let f = write_temp("");
    let (raw, _, had_errors) = encoding_rs::SHIFT_JIS.encode("｜日本《にほん》");
    assert!(!had_errors);
    fs::write(f.path(), raw.as_ref()).expect("seed sjis");
    let path = f.path().to_str().unwrap();
    let (status, stdout, stderr) = run(&["fmt", "--write", "-E", "sjis", path], None);
    assert!(status.success(), "fmt --write must succeed: {stderr:?}");
    assert_eq!(stdout, "");
    let (expected, _, had_errors) = encoding_rs::SHIFT_JIS.encode("日本《にほん》");
    assert!(!had_errors);
    assert_eq!(fs::read(path).expect("read back"), expected.as_ref());
}

#[test]
fn fmt_list_reports_only_dirty_stdin() {
    let (dirty_status, dirty_stdout, dirty_stderr) =
        run(&["fmt", "--list"], Some("｜日本《にほん》"));
    assert!(
        dirty_status.success(),
        "dirty stdin is informational: {dirty_stderr:?}"
    );
    assert_eq!(dirty_stdout, "<stdin>\n");

    let (clean_status, clean_stdout, clean_stderr) =
        run(&["fmt", "--list"], Some("日本《にほん》\n"));
    assert!(
        clean_status.success(),
        "canonical stdin succeeds: {clean_stderr:?}"
    );
    assert_eq!(clean_stdout, "");
}

#[test]
fn fmt_list_reports_a_dirty_file() {
    let file = write_temp("｜日本《にほん》");
    let path = file.path().to_str().expect("utf8 path");
    let (status, stdout, stderr) = run(&["fmt", "--list", path], None);
    assert!(status.success(), "--list is informational: {stderr:?}");
    assert_eq!(stdout, format!("{path}\n"));
}

#[test]
fn fmt_diff_prints_unified_diff() {
    let (status, stdout, _) = run(
        &["fmt", "--diff", "--color", "never"],
        Some("｜日本《にほん》"),
    );
    assert!(!status.success(), "--diff on dirty input exits non-zero");
    assert!(
        stdout.contains("@@") && stdout.contains("-｜日本") && stdout.contains("+日本"),
        "expected a unified diff hunk: {stdout:?}"
    );
}

#[test]
fn pandoc_without_output_format_emits_json() {
    let (status, stdout, stderr) = run(&["pandoc"], Some("青空\n"));
    assert!(status.success(), "pandoc projection failed: {stderr:?}");
    let value: serde_json::Value = serde_json::from_str(&stdout).expect("Pandoc JSON");
    assert!(value.get("pandoc-api-version").is_some());
    assert!(value.get("blocks").is_some());
}

#[test]
fn pandoc_json_emits_resolved_gaiji_as_unicode() {
    let source = "※［＃「木＋吶のつくり」、第3水準1-85-54］";
    let (status, stdout, stderr) = run(&["pandoc"], Some(source));
    assert!(status.success(), "pandoc projection failed: {stderr:?}");
    assert!(stdout.contains(r#""c":"枘""#), "Pandoc JSON: {stdout:?}");
    assert!(!stdout.contains("Char("), "Pandoc JSON: {stdout:?}");
    assert!(!stdout.contains("Multi("), "Pandoc JSON: {stdout:?}");
}

#[cfg(unix)]
#[test]
fn pandoc_child_stdin_broken_pipe_is_not_silent_success() {
    use std::os::unix::fs::PermissionsExt;

    let fake_bin = tempfile::tempdir().expect("fake bin dir");
    let fake_pandoc = fake_bin.path().join("pandoc");
    fs::write(&fake_pandoc, "#!/bin/sh\nexec 0<&-\nsleep 1\nexit 17\n").expect("fake pandoc");
    fs::set_permissions(&fake_pandoc, fs::Permissions::from_mode(0o755))
        .expect("executable fake pandoc");
    let mut paths = vec![fake_bin.path().to_path_buf()];
    paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let path = env::join_paths(paths).expect("test PATH");

    let mut child = common::hermetic_command()
        .args(["pandoc", "--to", "plain"])
        .env("PATH", path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aozora");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all("青空｜文庫《ぶんこ》\n".repeat(100_000).as_bytes())
        .expect("write source");
    let output = child.wait_with_output().expect("wait");
    assert_eq!(output.status.code(), Some(2));
}

#[cfg(unix)]
#[test]
fn pandoc_child_stdout_broken_pipe_is_silent_success() {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;

    let fake_bin = tempfile::tempdir().expect("fake bin dir");
    let fake_pandoc = fake_bin.path().join("pandoc");
    fs::write(
        &fake_pandoc,
        "#!/bin/sh\ncat >/dev/null\nexec yes aozora-pandoc-output\n",
    )
    .expect("fake pandoc");
    fs::set_permissions(&fake_pandoc, fs::Permissions::from_mode(0o755))
        .expect("executable fake pandoc");
    let mut paths = vec![fake_bin.path().to_path_buf()];
    paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let path = env::join_paths(paths).expect("test PATH");

    let mut child = common::hermetic_command()
        .args(["pandoc", "--to", "html"])
        .env("PATH", path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aozora");
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(b"aozora\n")
        .expect("write source");
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut prefix = [0_u8; 64];
    stdout.read_exact(&mut prefix).expect("read output prefix");
    drop(stdout);

    let output = child.wait_with_output().expect("wait");
    assert!(
        output.status.success(),
        "closed downstream stdout must be success: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stderr, Vec::<u8>::new());
}

#[test]
fn fmt_diff_is_silent_for_canonical_stdin() {
    let (status, stdout, stderr) = run(
        &["fmt", "--diff", "--color", "never"],
        Some("日本《にほん》\n"),
    );
    assert!(status.success(), "canonical input must pass: {stderr:?}");
    assert!(stdout.is_empty(), "canonical input has no diff: {stdout:?}");
}

#[test]
fn fmt_diff_on_a_file_uses_the_diff_reporter() {
    let file = write_temp("｜日本《にほん》");
    let path = file.path().to_str().expect("utf8 path");
    let (status, stdout, stderr) = run(&["fmt", "--diff", "--color", "never", path], None);
    assert!(!status.success(), "--diff on dirty input exits non-zero");
    assert!(
        stdout.contains("@@") && stdout.contains("-｜日本") && stdout.contains("+日本"),
        "expected a unified diff hunk: {stdout:?}"
    );
    assert!(
        !stderr.contains("would be reformatted"),
        "diff mode must not use the check-only reporter: {stderr:?}"
    );
}

#[test]
fn fmt_check_on_a_file_reports_dirty_input() {
    let file = write_temp("｜日本《にほん》");
    let path = file.path().to_str().expect("utf8 path");
    let (status, stdout, stderr) = run(&["fmt", "--check", path], None);
    assert!(!status.success(), "--check on dirty input exits non-zero");
    assert!(stdout.is_empty(), "plain check has no stdout: {stdout:?}");
    assert!(
        stderr.contains("would be reformatted"),
        "plain check reports the file: {stderr:?}"
    );
}

#[test]
fn fmt_encoding_sjis_decodes_shift_jis() {
    let f = write_temp("");
    fs::write(f.path(), [0x93, 0xfa, 0x96, 0x7b]).expect("seed sjis");
    let path = f.path().to_str().unwrap();
    let (status, stdout, stderr) = run(&["fmt", "--encoding", "sjis", path], None);
    assert!(status.success(), "sjis decode must succeed: {stderr:?}");
    assert!(stdout.contains("日本"), "expected decoded 日本: {stdout:?}");
}

#[test]
fn fmt_fix_rewrites_flagged_directive_near_miss() {
    let (status, stdout, _) = run(&["fmt", "--fix"], Some("あ［＃字下げ終わり］"));
    assert!(status.success(), "fmt --fix must succeed");
    assert!(
        stdout.contains("ここで字下げ終わり"),
        "--fix should canonicalise the directive: {stdout:?}"
    );
}

#[test]
fn lint_reports_non_canonical_directive() {
    let (_, _, stderr) = run(&["lint", "--format", "short"], Some("あ［＃字下げ終わり］"));
    assert!(
        stderr.contains("aozora::lint::non_canonical_directive"),
        "lint must flag the near-miss: {stderr:?}"
    );
    assert!(
        stderr.contains("ここで字下げ終わり"),
        "lint must suggest the canonical form: {stderr:?}"
    );
}

#[test]
fn lint_json_diagnostics_are_written_only_to_stderr() {
    let (status, stdout, stderr) = run(&["lint", "--format", "json"], Some("あ［＃字下げ終わり］"));
    assert!(status.success(), "non-strict lint remains successful");
    assert!(stdout.is_empty(), "diagnostic JSON must not use stdout");
    let report: serde_json::Value = serde_json::from_str(&stderr).expect("diagnostic JSON");
    assert_eq!(report["schemaVersion"], SCHEMA_VERSION);
    assert_eq!(report["data"][0]["kind"], "non_canonical_directive");
}

#[test]
fn lint_clean_input_is_silent_and_succeeds() {
    let (status, stdout, stderr) = run(&["lint"], Some("ただの本文"));
    assert!(status.success(), "clean input exits 0: {stderr:?}");
    assert!(stdout.is_empty() && stderr.is_empty(), "lint stays silent");
}

#[test]
fn lint_strict_exits_non_zero_when_a_lint_fires() {
    let (status, _, _) = run(&["lint", "--strict"], Some("あ［＃字下げ終わり］"));
    assert!(!status.success(), "--strict must exit 1 on a lint");
}

#[test]
fn lint_ignores_non_lint_lex_faults() {
    let (_, _, stderr) = run(&["lint", "--format", "short"], Some("あ［＃"));
    assert!(
        !stderr.contains("aozora::lint::"),
        "lint must not surface lex faults: {stderr:?}"
    );
}

#[test]
fn lint_fix_rewrites_file_then_relints_clean() {
    let f = write_temp("あ［＃字下げ終わり］");
    let path = f.path().to_str().unwrap();
    let (status, _, stderr) = run(&["lint", "--fix", path], None);
    assert!(status.success(), "lint --fix must succeed: {stderr:?}");
    let written = fs::read_to_string(path).expect("read back");
    assert!(
        written.contains("ここで字下げ終わり"),
        "lint --fix must canonicalise in place: {written:?}"
    );
    let (status, _, _) = run(&["lint", "--strict", path], None);
    assert!(status.success(), "fixed file must re-lint clean");
}

#[test]
fn lint_fix_preserves_auto_detected_shift_jis_encoding() {
    let f = write_temp("");
    let (raw, _, had_errors) = encoding_rs::SHIFT_JIS.encode("あ［＃字下げ終わり］");
    assert!(!had_errors);
    fs::write(f.path(), raw.as_ref()).expect("seed sjis");
    let path = f.path().to_str().unwrap();
    let (status, _, stderr) = run(&["lint", "--fix", path], None);
    assert!(status.success(), "lint --fix must succeed: {stderr:?}");
    let (expected, _, had_errors) =
        encoding_rs::SHIFT_JIS.encode("あ\n\n［＃ここで字下げ終わり］\n\n");
    assert!(!had_errors);
    assert_eq!(fs::read(path).expect("read back"), expected.as_ref());
}

#[test]
fn lint_fix_on_stdin_is_a_usage_error() {
    let (status, _, stderr) = run(&["lint", "--fix"], Some("あ"));
    assert!(!status.success(), "lint --fix on stdin must fail");
    assert!(
        stderr.contains("cannot rewrite stdin"),
        "expected a stdin hint: {stderr:?}"
    );
}

#[test]
fn render_emits_html_with_paragraph_tags() {
    let (status, stdout, _) = run(&["render"], Some("Hello.\n"));
    assert!(status.success(), "render should succeed");
    assert_eq!(stdout, "<p>Hello.</p>\n");
}

#[test]
fn render_emits_ruby_markup_for_explicit_delimiter() {
    let (status, stdout, _) = run(&["render"], Some("｜青梅《おうめ》\n"));
    assert!(status.success());
    assert!(stdout.contains("<ruby>青梅"), "missing ruby: {stdout:?}");
    assert!(stdout.contains("<rt>おうめ"), "missing rt: {stdout:?}");
}

#[test]
fn render_does_not_leak_pua_sentinels() {
    let (_, stdout, _) = run(&["render"], Some("｜青梅《おうめ》"));
    for pua in &['\u{E001}', '\u{E002}', '\u{E003}', '\u{E004}'] {
        assert!(
            !stdout.contains(*pua),
            "PUA sentinel U+{:04X} leaked into render output",
            *pua as u32,
        );
    }
}

#[test]
fn render_rejects_non_utf8_input_when_encoding_is_utf8() {
    let sjis_bytes: Vec<u8> = vec![0x82, 0xa0];
    let mut child = common::hermetic_command()
        .args(["render", "-E", "utf8"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(&sjis_bytes)
        .expect("write");
    let output = child.wait_with_output().expect("wait");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(2),
        "invalid UTF-8 is an input error"
    );
    assert!(
        stderr.contains("UTF-8") || stderr.contains("utf-8"),
        "expected encoding hint on stderr: {stderr:?}"
    );
}

#[test]
fn document_commands_map_unreadable_input_to_usage_exit() {
    for args in [
        &["check", "missing.aozora"][..],
        &["lint", "missing.aozora"][..],
        &["render", "missing.aozora"][..],
        &["inspect", "nodes", "missing.aozora"][..],
        &["pandoc", "missing.aozora"][..],
        &["fmt", "missing.aozora"][..],
    ] {
        let output = common::hermetic_command()
            .args(args)
            .output()
            .expect("run aozora");
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?} must classify an unreadable input as a usage error: {:?}",
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

#[test]
fn render_accepts_sjis_input_with_explicit_encoding_flag() {
    let sjis_bytes: Vec<u8> = vec![0x82, 0xa0, 0x82, 0xa2, 0x82, 0xa4, 0x82, 0xa6, 0x82, 0xa8];
    let mut child = common::hermetic_command()
        .args(["render", "-E", "sjis"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(&sjis_bytes)
        .expect("write");
    let output = child.wait_with_output().expect("wait");
    assert!(
        output.status.success(),
        "sjis decode + render must succeed: stderr={:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("あいうえお"),
        "decoded text missing from render: {stdout:?}"
    );
}

#[cfg(unix)]
fn oversized_source() -> String {
    "｜青梅《おうめ》\n".repeat(50_000)
}

#[cfg(unix)]
fn broken_pipe_run(args: &[&str], input_path: &str) -> (ExitStatus, String) {
    use std::io::Read;

    let mut child = common::hermetic_command()
        .args(args)
        .arg(input_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aozora");

    {
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut buf = [0u8; 64];
        let _n = stdout.read(&mut buf);
    }

    let output = child.wait_with_output().expect("wait");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    (output.status, stderr)
}

#[cfg(unix)]
#[test]
fn render_exits_zero_and_silent_on_broken_pipe() {
    let f = write_temp(&oversized_source());
    let (status, stderr) = broken_pipe_run(&["render"], f.path().to_str().unwrap());
    assert!(
        status.success(),
        "render into a closed pipe must exit 0: status={status:?} stderr={stderr:?}"
    );
    assert!(
        stderr.is_empty(),
        "a broken pipe must stay silent on stderr: {stderr:?}"
    );
}

#[cfg(unix)]
#[test]
fn inspect_exits_zero_and_silent_on_broken_pipe() {
    let f = write_temp(&oversized_source());
    let (status, stderr) = broken_pipe_run(&["inspect", "nodes"], f.path().to_str().unwrap());
    assert!(
        status.success(),
        "inspect into a closed pipe must exit 0: status={status:?} stderr={stderr:?}"
    );
    assert!(
        stderr.is_empty(),
        "a broken pipe must stay silent on stderr: {stderr:?}"
    );
}

#[cfg(unix)]
#[test]
fn fmt_exits_zero_and_silent_on_broken_pipe() {
    let f = write_temp(&oversized_source());
    let (status, stderr) = broken_pipe_run(&["fmt"], f.path().to_str().unwrap());
    assert!(
        status.success(),
        "fmt into a closed pipe must exit 0: status={status:?} stderr={stderr:?}"
    );
    assert!(
        stderr.is_empty(),
        "a broken pipe must stay silent on stderr: {stderr:?}"
    );
}

#[cfg(unix)]
#[test]
fn stderr_broken_pipe_is_an_operational_error() {
    let mut child = common::hermetic_command()
        .args(["check", "--strict", "--format", "json", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aozora");

    drop(child.stderr.take());
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all("bad \u{E001} char\n".as_bytes())
        .expect("write diagnostic source");

    let status = child.wait().expect("wait");
    assert_eq!(status.code(), Some(2));
}

#[test]
fn render_auto_detects_sjis_without_encoding_flag() {
    let sjis_bytes: Vec<u8> = vec![0x82, 0xa0, 0x82, 0xa2, 0x82, 0xa4, 0x82, 0xa6, 0x82, 0xa8];
    let mut child = common::hermetic_command()
        .args(["render"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(&sjis_bytes)
        .expect("write");
    let output = child.wait_with_output().expect("wait");
    assert!(
        output.status.success(),
        "auto-detect must decode + render SJIS: stderr={:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("あいうえお"),
        "auto-detected text missing from render"
    );
}

#[test]
fn repl_evaluates_piped_lines_then_quits() {
    let script = "｜青空《あおぞら》\n:mode html\n青梅《おうめ》\n:quit\n";
    let (status, stdout, stderr) = run(&["repl"], Some(script));
    assert!(
        status.success(),
        "scripted repl exits 0: {status:?} {stderr}"
    );
    assert!(stdout.contains(":help"), "banner shown: {stdout}");
    assert!(stdout.contains("青空"), "first line evaluated: {stdout}");
    assert!(
        stdout.contains("青梅"),
        "second line evaluated after :mode: {stdout}"
    );
}

#[test]
fn repl_surfaces_diagnostics_inline() {
    let script = "bad \u{E001} char\n:quit\n";
    let (status, stdout, stderr) = run(&["repl"], Some(script));
    assert!(
        status.success(),
        "repl exits 0 even with diagnostics: {stderr}"
    );
    assert!(
        stdout.contains("aozora::"),
        "diagnostic code shown inline: {stdout}"
    );
}

#[test]
fn tui_without_a_terminal_refuses_with_an_actionable_error() {
    let (status, _stdout, stderr) = run(&["tui"], None);
    assert!(
        !status.success(),
        "tui must refuse a non-terminal: {status:?} {stderr}"
    );
    assert!(
        stderr.contains("terminal"),
        "error names the missing terminal: {stderr}"
    );
}
