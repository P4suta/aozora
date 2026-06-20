//! `aozora completions <shell>` smoke tests — every supported shell
//! emits a non-empty script, and an unknown shell is rejected.
//!
//! Pure stdlib (mirrors `smoke.rs`). The script *contents* are clap's
//! responsibility; here we only confirm the subcommand is wired and
//! every shell dialect dispatches.

use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_aozora");

fn run(args: &[&str]) -> (bool, String) {
    let out = Command::new(BIN)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("spawn aozora");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn every_supported_shell_emits_a_script() {
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let (ok, stdout) = run(&["completions", shell]);
        assert!(ok, "completions {shell} exited non-zero");
        assert!(
            !stdout.trim().is_empty(),
            "completions {shell} produced an empty script"
        );
        assert!(
            stdout.contains("aozora"),
            "completions {shell} script missing the binary name"
        );
    }
}

#[test]
fn unknown_shell_is_rejected() {
    let (ok, _) = run(&["completions", "tcsh"]);
    assert!(!ok, "an unsupported shell must exit non-zero");
}
