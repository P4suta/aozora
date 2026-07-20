//! Integration coverage for `--watch`.

use std::io::Write;
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

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
    let deadline = Instant::now() + Duration::from_secs(2);
    while child.try_wait().expect("poll aozora").is_none() {
        if Instant::now() >= deadline {
            child.kill().expect("stop hung aozora");
            child.wait().expect("reap hung aozora");
            panic!("--watch on stdin entered the watch loop");
        }
        thread::sleep(Duration::from_millis(10));
    }
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

    for iteration in 0..30 {
        let source = if iteration % 2 == 0 {
            "｜赤《あか》"
        } else {
            "｜青《あお》"
        };
        fs::write(&path, source).expect("edit input");
        sleep(Duration::from_millis(100));
    }
    child.kill().expect("kill watch");
    let output = child.wait_with_output().expect("wait for aozora");
    let stdout = String::from_utf8_lossy(&output.stdout);
    // The initial render plus the post-edit render both reached stdout.
    assert!(
        stdout.matches("ruby").count() >= 2,
        "watch re-rendered on change: {stdout:?}"
    );
}
