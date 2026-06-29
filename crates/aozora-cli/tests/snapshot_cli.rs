//! Snapshot the CLI's user-visible surface so changes to `--help`,
//! `--version`, and subcommand output land as intentional review
//! diffs instead of silent drift.
//!
//! Uses `std::process::Command` + the `CARGO_BIN_EXE_aozora`
//! environment variable that Cargo provides for integration tests
//! against binary targets — no extra dev-dep needed (no
//! `assert_cmd`, no `escargot`).
//!
//! `insta` filters mask the runtime version string so a routine
//! workspace `version` bump does not require accepting a new
//! snapshot for `--version`.

use std::process::Command;

fn run(args: &[&str]) -> String {
    let bin = env!("CARGO_BIN_EXE_aozora");
    let output = Command::new(bin)
        .args(args)
        .output()
        .expect("failed to spawn aozora CLI");
    let stdout = String::from_utf8(output.stdout).expect("CLI stdout is UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("CLI stderr is UTF-8");
    if stderr.is_empty() {
        stdout
    } else {
        format!("{stdout}\n----- STDERR -----\n{stderr}")
    }
}

#[test]
fn snapshot_top_level_help() {
    insta::with_settings!({
        // Snapshot is independent of the binary path / version string.
        filters => vec![
            (r"\d+\.\d+\.\d+(?:-[\w.]+)?", "[VERSION]"),
        ],
    }, {
        insta::assert_snapshot!(run(&["--help"]));
    });
}

#[test]
fn snapshot_version_flag() {
    insta::with_settings!({
        filters => vec![
            // Redact the full channel-aware stamp: core triple + optional
            // pre-release (`-dev` / `-nightly.<date>`) + optional build
            // metadata (`+g<sha>` / `+g<sha>.dirty`). Without the `\+…` arm a
            // working-copy `dev` build leaks its commit sha and the snapshot is
            // sha-dependent. See aozora-buildstamp.
            (r"\d+\.\d+\.\d+(?:-[\w.]+)?(?:\+[\w.]+)?", "[VERSION]"),
        ],
    }, {
        insta::assert_snapshot!(run(&["--version"]));
    });
}

#[test]
fn snapshot_check_help() {
    insta::with_settings!({
        filters => vec![
            (r"\d+\.\d+\.\d+(?:-[\w.]+)?", "[VERSION]"),
        ],
    }, {
        insta::assert_snapshot!(run(&["check", "--help"]));
    });
}

#[test]
fn snapshot_fmt_help() {
    insta::with_settings!({
        filters => vec![
            (r"\d+\.\d+\.\d+(?:-[\w.]+)?", "[VERSION]"),
        ],
    }, {
        insta::assert_snapshot!(run(&["fmt", "--help"]));
    });
}

#[test]
fn snapshot_inspect_help() {
    insta::with_settings!({
        filters => vec![
            (r"\d+\.\d+\.\d+(?:-[\w.]+)?", "[VERSION]"),
        ],
    }, {
        insta::assert_snapshot!(run(&["inspect", "--help"]));
    });
}

#[test]
fn snapshot_completions_help() {
    insta::with_settings!({
        filters => vec![
            (r"\d+\.\d+\.\d+(?:-[\w.]+)?", "[VERSION]"),
        ],
    }, {
        insta::assert_snapshot!(run(&["completions", "--help"]));
    });
}

#[test]
fn snapshot_explain_help() {
    insta::with_settings!({
        filters => vec![
            (r"\d+\.\d+\.\d+(?:-[\w.]+)?", "[VERSION]"),
        ],
    }, {
        insta::assert_snapshot!(run(&["explain", "--help"]));
    });
}
