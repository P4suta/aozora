//! End-to-end tests for `aozora lsp` — the exec-delegate to the `aozora-lsp`
//! daemon. Two behaviours the unit tests in `src/lsp.rs` cannot reach through
//! a spawned process: a real hand-off to a stub daemon on `PATH` (argv is
//! forwarded verbatim), and the actionable exit-2 error when no daemon is
//! installed.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use tempfile::TempDir;

/// The `aozora` binary under test.
const BIN: &str = env!("CARGO_BIN_EXE_aozora");

/// Pin the message language and seal the global config layer at an empty XDG
/// dir, so these runs stay deterministic regardless of the host locale or a
/// developer's real `~/.config/aozora`. Kept local (rather than the shared
/// `common` harness) so both tests — one of which is Unix-only — always
/// exercise it and no import is left unused on any platform.
fn hermetic(cmd: &mut Command, xdg: &Path) {
    cmd.env("AOZORA_LANG", "en")
        .env("XDG_CONFIG_HOME", xdg)
        .env_remove("LANG")
        .env_remove("LC_ALL");
}

/// A directory holding an executable `aozora-lsp` stub that echoes a marker
/// plus the argv it received, so a test can prove the hand-off happened and
/// that arguments were forwarded untouched.
#[cfg(unix)]
fn stub_dir() -> TempDir {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = TempDir::new().expect("stub dir");
    let stub = dir.path().join("aozora-lsp");
    fs::write(&stub, b"#!/bin/sh\nprintf 'stub-lsp:%s\\n' \"$*\"\n").expect("write stub");
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).expect("chmod +x");
    dir
}

#[cfg(unix)]
#[test]
fn lsp_delegates_to_the_daemon_forwarding_argv() {
    let xdg = TempDir::new().expect("xdg dir");
    let stub = stub_dir();
    // PATH holds only the stub dir, so the delegate resolves the stub (PATH is
    // searched before the sibling-of-binary fallback) regardless of any real
    // `aozora-lsp` in the build output directory.
    let mut cmd = Command::new(BIN);
    hermetic(&mut cmd, xdg.path());
    let output = cmd
        .args(["lsp", "--stdio"])
        .env("PATH", stub.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn aozora lsp");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "delegation exits 0: {output:?}");
    assert!(
        stdout.contains("stub-lsp:"),
        "the daemon stub ran (its marker is on stdout): {stdout:?}"
    );
    assert!(
        stdout.contains("--stdio"),
        "argv is forwarded verbatim to the daemon: {stdout:?}"
    );
}

#[test]
fn lsp_missing_daemon_is_an_actionable_exit_2() {
    // Copy the binary into an isolated dir so the "next to this binary"
    // fallback has no `aozora-lsp` sibling to find, then run it with no PATH.
    // Neither lookup can succeed, so this exercises the not-installed path.
    let xdg = TempDir::new().expect("xdg dir");
    let home = TempDir::new().expect("isolated home");
    let exe = home.path().join(exe_name());
    fs::copy(BIN, &exe).expect("copy the aozora binary");
    make_executable(&exe);

    let mut cmd = Command::new(&exe);
    hermetic(&mut cmd, xdg.path());
    let output = cmd
        .args(["lsp", "--stdio"])
        .env_remove("PATH")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn the isolated aozora binary");

    assert_eq!(
        output.status.code(),
        Some(2),
        "a missing daemon is a usage error (exit 2), not a generic failure: {output:?}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("aozora-lsp") && stderr.contains("install"),
        "the error names the daemon and how to fix it: {stderr:?}"
    );
}

/// The file name to copy the binary under, preserving the platform extension
/// (`aozora.exe` on Windows) so the OS still recognises it as executable.
fn exe_name() -> String {
    Path::new(BIN)
        .file_name()
        .expect("binary has a file name")
        .to_string_lossy()
        .into_owned()
}

/// Restore the exec bit that `fs::copy` may not carry across on Unix; a no-op
/// elsewhere, where an `.exe` is executable by extension.
#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("preserve exec bit");
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}
