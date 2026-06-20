//! `aozora man` smoke tests — the hidden man-page generator emits roff
//! for the root command and for named subcommands, fails on an unknown
//! one, and stays hidden from `--help`.

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
fn man_root_emits_roff_for_the_binary() {
    let (ok, stdout) = run(&["man"]);
    assert!(ok, "man (root) must succeed");
    assert!(
        stdout.contains(".TH aozora"),
        "expected a roff .TH header: {stdout:?}"
    );
    assert!(
        stdout.contains("Aozora Bunko notation parser CLI"),
        "missing the NAME section"
    );
}

#[test]
fn man_renders_a_named_subcommand() {
    let (ok, stdout) = run(&["man", "inspect"]);
    assert!(ok, "man wire must succeed");
    assert!(stdout.contains(".TH"), "expected roff output");
    assert!(
        stdout.contains("inspect"),
        "subcommand page missing its name"
    );
}

#[test]
fn man_unknown_subcommand_fails() {
    let (ok, _) = run(&["man", "does-not-exist"]);
    assert!(!ok, "unknown subcommand must exit non-zero");
}

#[test]
fn man_is_hidden_from_help_but_runnable() {
    let (help_ok, help) = run(&["--help"]);
    assert!(help_ok);
    assert!(
        !help.lines().any(|l| l.trim_start().starts_with("man ")),
        "man must be hidden from --help: {help:?}"
    );
    let (man_ok, _) = run(&["man"]);
    assert!(man_ok, "man must still be runnable while hidden");
}
