//! End-to-end smoke tests for the `aozora` binary.
//!
//! Each test spawns the *built* binary (resolved via the
//! `CARGO_BIN_EXE_aozora` env var that `cargo test` injects) so we
//! exercise the actual `ExitCode` + argv plumbing alongside the
//! library, not just the library API.
//!
//! What this catches that the library tests can't:
//! - argv / clap-derive wiring (subcommand dispatch, flag parsing)
//! - encoding flag (`--encoding sjis`) byte path
//! - stdin handling (`-` and missing positional)
//! - exit codes (0 / non-zero) — fundamental for shell composition
//! - real-file vs stdin-pipe behaviour parity
//!
//! Pure stdlib so the test crate stays dep-light. `assert_cmd` would
//! be a step up if the suite grows; for now `Command` reads cleanly.

use std::fs;
use std::io::Write;
use std::process::{Command, ExitStatus, Stdio};

use tempfile::NamedTempFile;

const BIN: &str = env!("CARGO_BIN_EXE_aozora");

fn write_temp(contents: &str) -> NamedTempFile {
    let mut f = tempfile::Builder::new()
        .prefix("aozora-cli-test-")
        .suffix(".txt")
        .tempfile()
        .expect("temp file");
    f.write_all(contents.as_bytes()).expect("write temp");
    f.flush().expect("flush temp");
    f
}

/// Run the binary with `args`, optionally feeding `stdin`. Returns
/// (status, stdout, stderr) — every smoke test should assert on at
/// least the exit status, and one of stdout/stderr to ensure the
/// path actually executed (not just compiled).
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
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    (output.status, stdout, stderr)
}

// ---------------------------------------------------------------------
// `aozora --version` — basic spawn / clap wiring smoke
// ---------------------------------------------------------------------

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
    // clap's default behaviour: missing required subcommand → 2.
    let (status, _, stderr) = run(&[], None);
    assert!(!status.success(), "expected failure on missing subcommand");
    assert!(
        stderr.contains("Usage:") || stderr.contains("usage:"),
        "expected clap help banner, got: {stderr:?}"
    );
}

// ---------------------------------------------------------------------
// `aozora check` — diagnostics path
// ---------------------------------------------------------------------

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
    // Source containing a literal PUA sentinel triggers a
    // `SourceContainsPua` warning (and an `UnregisteredSentinel`
    // error). Without `--strict` we still exit 0 but print the
    // diagnostic; with `--strict` we exit non-zero.
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

// ---------------------------------------------------------------------
// `aozora fmt` — round-trip path
// ---------------------------------------------------------------------

#[test]
fn fmt_default_prints_canonical_form_on_stdout() {
    // Implicit-delimiter ruby canonicalises to explicit; stdout must
    // carry the canonical form.
    let (status, stdout, _) = run(&["fmt"], Some("日本《にほん》"));
    assert!(status.success(), "fmt should succeed");
    assert!(
        stdout.contains('｜'),
        "expected explicit delimiter in canonical form: {stdout:?}"
    );
}

#[test]
fn fmt_check_succeeds_on_already_canonical_input() {
    // Canonical input → stdout silent, exit 0.
    let canonical = "｜青梅《おうめ》\n";
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
fn fmt_check_fails_on_non_canonical_input() {
    let non_canonical = "日本《にほん》"; // missing explicit ｜
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
    let f = write_temp("日本《にほん》");
    let path = f.path().to_str().unwrap();
    let (status, _, stderr) = run(&["fmt", "--write", path], None);
    assert!(status.success(), "fmt --write must succeed: {stderr:?}");
    let written = fs::read_to_string(path).expect("read back");
    assert!(
        written.contains('｜'),
        "file must contain canonical output: {written:?}"
    );
}

// ---------------------------------------------------------------------
// `aozora render` — HTML output path
// ---------------------------------------------------------------------

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

// ---------------------------------------------------------------------
// Encoding flag — UTF-8 vs Shift_JIS
// ---------------------------------------------------------------------

#[test]
fn render_rejects_non_utf8_input_when_encoding_is_utf8() {
    // A raw SJIS byte sequence is not valid UTF-8; without
    // `-E sjis`, the binary must report the input as malformed
    // rather than silently producing garbage.
    let sjis_bytes: Vec<u8> = vec![0x82, 0xa0]; // 「あ」 in SJIS
    let mut child = Command::new(BIN)
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
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "must reject invalid UTF-8");
    assert!(
        stderr.contains("UTF-8") || stderr.contains("utf-8"),
        "expected encoding hint on stderr: {stderr:?}"
    );
}

#[test]
fn render_accepts_sjis_input_with_explicit_encoding_flag() {
    // 「あいうえお」 in Shift_JIS.
    let sjis_bytes: Vec<u8> = vec![0x82, 0xa0, 0x82, 0xa2, 0x82, 0xa4, 0x82, 0xa6, 0x82, 0xa8];
    let mut child = Command::new(BIN)
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
