//! Spawns the compiled `aozora-lsp` binary to cover the argv surface
//! (`Cli::parse_args` → clap), which a unit test can't reach without
//! touching the test runner's own argv. clap prints and exits for
//! `--version` / `--help` (and on a usage error) *before* `run` opens the
//! stdio JSON-RPC stream, so none of these spawns boot the server or hang.
//!
//! NB: this test is deliberately *not* gated on the `internals` feature —
//! it drives the public binary, not the internal API.

use std::process::Command;

fn aozora_lsp() -> Command {
    Command::new(env!("CARGO_BIN_EXE_aozora-lsp"))
}

#[test]
fn version_flag_prints_version_and_exits_zero() {
    let out = aozora_lsp()
        .arg("--version")
        .output()
        .expect("spawn --version");
    assert!(out.status.success(), "exit: {:?}", out.status);
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    // In the monorepo the crate version *is* the parser version, so
    // `--version` reports it plainly (e.g. `aozora-lsp 0.4.1`).
    assert!(
        stdout.contains(env!("CARGO_PKG_VERSION")),
        "version output should carry the crate version: {stdout:?}",
    );
}

#[test]
fn help_flag_describes_stdio_transport_and_exits_zero() {
    let out = aozora_lsp().arg("--help").output().expect("spawn --help");
    assert!(out.status.success(), "exit: {:?}", out.status);
    let stdout = String::from_utf8(out.stdout).expect("utf8");
    assert!(
        stdout.contains("stdio"),
        "help should mention the stdio transport: {stdout:?}",
    );
}

#[test]
fn unknown_flag_is_a_usage_error() {
    let out = aozora_lsp()
        .arg("--definitely-not-a-flag")
        .output()
        .expect("spawn bad flag");
    assert!(
        !out.status.success(),
        "an unknown flag must exit non-zero, got: {:?}",
        out.status,
    );
}
