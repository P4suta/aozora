//! Integration coverage for `--watch` (src/watch.rs).
//!
//! The decisive, deterministic check is that `--watch` on stdin is a
//! usage error — a real watch loop is non-deterministic and long-running,
//! so the end-to-end re-run test is `#[ignore]`d (run it by name with
//! `cargo test -- --ignored`).

use std::io::Write;
use std::process::Stdio;

mod common;

#[test]
fn watch_on_stdin_is_a_usage_error() {
    // `--watch -` cannot watch a pipe; it must fail fast with exit 2 and
    // never enter the watch loop.
    let mut child = common::hermetic_command()
        .args(["check", "--watch", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn aozora");
    // Close stdin so the child cannot block on a read.
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait for aozora");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(2),
        "--watch on stdin → exit 2: {stderr}"
    );
    assert!(
        stderr.contains("cannot watch stdin"),
        "error explains why: {stderr}"
    );
}

#[test]
#[ignore = "long-running watch loop; run explicitly with --ignored"]
fn watch_reruns_on_change() {
    use std::fs;
    use std::thread::sleep;
    use std::time::Duration;

    use tempfile::Builder;

    let mut file = Builder::new()
        .prefix("aozora-watch-")
        .suffix(".txt")
        .tempfile()
        .expect("temp file");
    write!(file, "｜青《あ》").expect("seed input");
    file.flush().expect("flush");
    let path = file.path().to_owned();

    let mut child = common::hermetic_command()
        .args(["render", "--watch", path.to_str().expect("utf-8 path")])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn aozora");

    sleep(Duration::from_millis(300));
    fs::write(&path, "｜赤《あか》").expect("edit input");
    sleep(Duration::from_millis(600));
    child.kill().expect("kill watch");
    let output = child.wait_with_output().expect("wait for aozora");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // The initial render plus the post-edit render both reached stdout.
    assert!(
        stdout.matches("ruby").count() >= 2,
        "watch re-rendered on change: {stdout:?}"
    );
}
