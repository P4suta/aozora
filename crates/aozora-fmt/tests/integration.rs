//! Integration tests that shell out to the compiled `aozora-fmt` binary.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};

fn aozora_fmt() -> Command {
    Command::new(env!("CARGO_BIN_EXE_aozora-fmt"))
}

/// Non-canonical ruby: `日本《にほん》` formats to `｜日本《にほん》`.
const DIRTY: &str = "日本《にほん》\n";
/// The canonical form of [`DIRTY`].
const DIRTY_CANONICAL: &str = "｜日本《にほん》\n";
/// Already-canonical plain text (a fixed point of the formatter).
const CLEAN: &str = "plain text\n";

#[test]
fn stdin_to_stdout_prints_canonical_form() {
    let mut child = aozora_fmt()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aozora-fmt");
    child
        .stdin
        .as_mut()
        .expect("stdin piped")
        .write_all("日本《にほん》".as_bytes())
        .unwrap();
    let out = child.wait_with_output().expect("wait");
    assert!(out.status.success(), "exit: {:?}", out.status);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.starts_with('｜'),
        "expected canonical explicit delimiter, got {stdout:?}",
    );
}

#[test]
fn check_on_canonical_input_exits_zero() {
    let mut child = aozora_fmt()
        .arg("--check")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aozora-fmt");
    child.stdin.as_mut().unwrap().write_all(b"hello\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "canonical input must pass --check");
}

#[test]
fn check_on_non_canonical_input_exits_one() {
    let mut child = aozora_fmt()
        .arg("--check")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aozora-fmt");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all("日本《にほん》".as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "non-canonical input must fail --check",
    );
}

/// Write `contents` to a unique temp file and return its path. Each
/// test owns its file (keyed by pid + a per-test `name`, so parallel
/// tests never collide) and removes it at the end.
fn temp_file(name: &str, contents: &str) -> PathBuf {
    let mut path = env::temp_dir();
    path.push(format!("aozora-fmt-it-{}-{name}", process::id()));
    fs::write(&path, contents).expect("write temp file");
    path
}

#[test]
fn write_canonicalizes_in_place_and_is_idempotent() {
    let path = temp_file("write-roundtrip", "日本《にほん》");

    // First --write rewrites the file to canonical form and exits 0.
    let out = aozora_fmt().arg("--write").arg(&path).output().unwrap();
    assert!(
        out.status.success(),
        "first --write failed: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let after_first = fs::read_to_string(&path).unwrap();
    assert!(
        after_first.starts_with('｜'),
        "expected canonical explicit delimiter, got {after_first:?}",
    );

    // Second --write must be a byte-identical no-op: the on-disk output
    // is a fixed point of the formatter. This exercises the happy path
    // of the pre-write idempotency guard.
    let out2 = aozora_fmt().arg("--write").arg(&path).output().unwrap();
    assert!(out2.status.success());
    let after_second = fs::read_to_string(&path).unwrap();
    assert_eq!(
        after_first, after_second,
        "second --write must leave the file byte-identical",
    );

    fs::remove_file(&path).ok();
}

#[test]
fn write_to_stdin_is_rejected_with_exit_two() {
    let mut child = aozora_fmt()
        .arg("--write")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aozora-fmt");
    child.stdin.as_mut().unwrap().write_all(b"hello").unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "--write without a file path must exit 2",
    );
}

// --- multi-file / directory / diff / list / json --------------------------

/// Create a fresh, empty scratch directory unique to this test (keyed by
/// pid + `name`, so parallel tests never collide).
fn temp_dir(name: &str) -> PathBuf {
    let mut path = env::temp_dir();
    path.push(format!("aozora-fmt-it-dir-{}-{name}", process::id()));
    fs::remove_dir_all(&path).ok();
    fs::create_dir_all(&path).expect("create scratch dir");
    path
}

/// Write `contents` to `dir/rel`, creating parent directories.
fn write_in(dir: &Path, rel: &str, contents: &str) -> PathBuf {
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dir");
    }
    fs::write(&path, contents).expect("write file");
    path
}

#[test]
fn check_diff_prints_unified_diff_and_exits_one() {
    let path = temp_file("diff-input", DIRTY);
    let out = aozora_fmt()
        .args(["--check", "--diff", "--color", "never"])
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "dirty --check exits 1");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with("--- "), "diff has a header: {stdout:?}");
    assert!(
        stdout.contains("-日本《にほん》"),
        "removed line: {stdout:?}"
    );
    assert!(
        stdout.contains("+｜日本《にほん》"),
        "added line: {stdout:?}"
    );
    fs::remove_file(&path).ok();
}

#[test]
fn diff_without_check_implies_check() {
    let path = temp_file("diff-implies-check", DIRTY);
    let out = aozora_fmt()
        .args(["--diff", "--color", "never"])
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "--diff implies --check");
    assert!(
        String::from_utf8(out.stdout)
            .unwrap()
            .contains("+｜日本《にほん》")
    );
    fs::remove_file(&path).ok();
}

#[test]
fn check_lists_every_dirty_file_not_just_the_first() {
    let dir = temp_dir("check-all-dirty");
    write_in(&dir, "a.afm", DIRTY);
    write_in(&dir, "b.afm", DIRTY);
    let out = aozora_fmt().arg("--check").arg(&dir).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("a.afm would be reformatted"), "{stderr:?}");
    assert!(stderr.contains("b.afm would be reformatted"), "{stderr:?}");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn write_directory_formats_only_aozora_sources_recursively() {
    let dir = temp_dir("write-dir");
    let a = write_in(&dir, "a.afm", DIRTY);
    let nested = write_in(&dir, "sub/c.afm", DIRTY);
    let txt = write_in(&dir, "note.txt", DIRTY); // not an aozora source

    let out = aozora_fmt().arg("--write").arg(&dir).output().unwrap();
    assert!(
        out.status.success(),
        "{:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read_to_string(&a).unwrap(), DIRTY_CANONICAL);
    assert_eq!(
        fs::read_to_string(&nested).unwrap(),
        DIRTY_CANONICAL,
        "recursion"
    );
    assert_eq!(
        fs::read_to_string(&txt).unwrap(),
        DIRTY,
        "non-source untouched"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn list_over_directory_prints_sorted_dirty_paths() {
    let dir = temp_dir("list-dir");
    write_in(&dir, "a.afm", DIRTY);
    write_in(&dir, "b.afm", CLEAN); // clean → not listed
    write_in(&dir, "sub/c.afm", DIRTY);

    let out = aozora_fmt().arg("--list").arg(&dir).output().unwrap();
    assert!(out.status.success(), "gofmt -l exits 0");
    let stdout = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "only the two dirty files: {lines:?}");
    assert!(lines[0].ends_with("a.afm"), "sorted first: {lines:?}");
    assert!(lines[1].ends_with("c.afm"), "sorted second: {lines:?}");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn list_and_write_combined_lists_and_rewrites() {
    let dir = temp_dir("list-write");
    let a = write_in(&dir, "a.afm", DIRTY);
    let out = aozora_fmt().args(["-l", "-w"]).arg(&dir).output().unwrap();
    assert!(out.status.success());
    assert!(
        String::from_utf8(out.stdout).unwrap().contains("a.afm"),
        "listed"
    );
    assert_eq!(
        fs::read_to_string(&a).unwrap(),
        DIRTY_CANONICAL,
        "rewritten"
    );
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn check_json_reports_per_file_status() {
    let dir = temp_dir("json");
    write_in(&dir, "a.afm", DIRTY);
    write_in(&dir, "b.afm", CLEAN);

    let out = aozora_fmt()
        .args(["--check", "--json"])
        .arg(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("\"version\": 1"), "{stdout}");
    assert!(stdout.contains("\"formatted\": false"), "{stdout}");
    assert!(stdout.contains("\"would_reformat\""), "{stdout}");
    assert!(stdout.contains("\"ok\""), "{stdout}");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn mixing_stdin_and_file_exits_two() {
    let path = temp_file("mix", CLEAN);
    let out = aozora_fmt()
        .arg("--check")
        .arg("-")
        .arg(&path)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8(out.stderr)
            .unwrap()
            .contains("cannot mix stdin")
    );
    fs::remove_file(&path).ok();
}

#[test]
fn multiple_paths_without_mode_exits_two() {
    let a = temp_file("concat-a", CLEAN);
    let b = temp_file("concat-b", CLEAN);
    let out = aozora_fmt().arg(&a).arg(&b).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8(out.stderr).unwrap().contains("refusing"));
    fs::remove_file(&a).ok();
    fs::remove_file(&b).ok();
}

#[test]
fn missing_path_does_not_abort_the_other_files() {
    let dirty = temp_file("survivor", DIRTY);
    let out = aozora_fmt()
        .arg("--check")
        .arg(&dirty)
        .arg(dirty.with_file_name("definitely-missing.afm"))
        .output()
        .unwrap();
    // Error (missing file) dominates the would-reformat outcome → exit 2…
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8(out.stderr).unwrap();
    // …but the valid file was still processed (not fail-fast).
    assert!(
        stderr.contains("would be reformatted"),
        "processed survivor: {stderr:?}"
    );
    assert!(
        stderr.contains("definitely-missing.afm"),
        "reported missing: {stderr:?}"
    );
    fs::remove_file(&dirty).ok();
}

#[test]
fn color_never_emits_no_ansi_and_always_emits_ansi() {
    let path = temp_file("color", DIRTY);
    let never = aozora_fmt()
        .args(["--diff", "--color", "never"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(!never.stdout.contains(&0x1b), "no ESC with --color never");

    let always = aozora_fmt()
        .args(["--diff", "--color", "always"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        always.stdout.contains(&0x1b),
        "ESC present with --color always"
    );
    fs::remove_file(&path).ok();
}

#[test]
fn write_directory_is_idempotent() {
    let dir = temp_dir("idempotent");
    let a = write_in(&dir, "a.afm", DIRTY);

    assert!(
        aozora_fmt()
            .arg("-w")
            .arg(&dir)
            .output()
            .unwrap()
            .status
            .success()
    );
    let first = fs::read_to_string(&a).unwrap();
    assert!(
        aozora_fmt()
            .arg("-w")
            .arg(&dir)
            .output()
            .unwrap()
            .status
            .success()
    );
    let second = fs::read_to_string(&a).unwrap();
    assert_eq!(
        first, second,
        "second --write must be a byte-identical no-op"
    );
    fs::remove_dir_all(&dir).ok();
}

/// Spawn `aozora-fmt ARGS`, feed `input` on stdin, and capture the output.
fn run_with_stdin(args: &[&str], input: &str) -> process::Output {
    let mut child = aozora_fmt()
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aozora-fmt");
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn stdin_list_prints_stdin_label_when_dirty() {
    let out = run_with_stdin(&["--list"], DIRTY);
    assert!(out.status.success(), "gofmt -l exits 0");
    assert_eq!(String::from_utf8(out.stdout).unwrap().trim(), "<stdin>");
}

#[test]
fn stdin_diff_prints_unified_diff() {
    let out = run_with_stdin(&["--diff", "--color", "never"], DIRTY);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8(out.stdout)
            .unwrap()
            .contains("+｜日本《にほん》")
    );
}

#[test]
fn stdin_check_json_reports_single_file() {
    let out = run_with_stdin(&["--check", "--json"], DIRTY);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("\"<stdin>\""), "{stdout}");
    assert!(stdout.contains("\"would_reformat\""), "{stdout}");
}
