//! Snapshot the CLI's user-visible surface so changes to `--help`,
//! `--version`, and subcommand output land as intentional review
//! diffs instead of silent drift.
//!
//! Uses `std::process::Command` + the `CARGO_BIN_EXE_aozora`
//! environment variable that Cargo provides for integration tests
//! against binary targets — no extra dev-dep needed (no
//! `assert_cmd`, no `escargot`).
//!
//! `insta` filters (see `cli_filters`) mask runtime/platform artifacts
//! so the golden files capture the *surface* and not incidental noise.

use std::process::Command;

/// Filters applied to every CLI snapshot so the golden files record the
/// user-visible surface (flags, subcommands, prose) and never runtime or
/// platform artifacts that would otherwise force a spurious re-accept:
///
/// - the version stamp — core triple + optional pre-release
///   (`-dev` / `-nightly.<date>`) + optional build metadata
///   (`+g<sha>` / `+g<sha>.dirty`) — so a workspace `version` bump or a
///   `dev` build's commit sha never invalidates a snapshot (see
///   `aozora-buildstamp`);
/// - the Windows `.exe` suffix clap prints in the `Usage:` line: the test
///   spawns `CARGO_BIN_EXE_aozora`, which is `aozora.exe` on Windows, so
///   clap derives `Usage: aozora.exe …` there — normalising it keeps the
///   surface identical across the platforms the cross-os matrix runs on.
fn cli_filters() -> Vec<(&'static str, &'static str)> {
    vec![
        (r"\d+\.\d+\.\d+(?:-[\w.]+)?(?:\+[\w.]+)?", "[VERSION]"),
        (r"aozora\.exe", "aozora"),
    ]
}

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
    insta::with_settings!({ filters => cli_filters() }, {
        insta::assert_snapshot!(run(&["--help"]));
    });
}

#[test]
fn snapshot_version_flag() {
    insta::with_settings!({ filters => cli_filters() }, {
        insta::assert_snapshot!(run(&["--version"]));
    });
}

#[test]
fn snapshot_check_help() {
    insta::with_settings!({ filters => cli_filters() }, {
        insta::assert_snapshot!(run(&["check", "--help"]));
    });
}

#[test]
fn snapshot_lint_help() {
    insta::with_settings!({ filters => cli_filters() }, {
        insta::assert_snapshot!(run(&["lint", "--help"]));
    });
}

#[test]
fn snapshot_fmt_help() {
    insta::with_settings!({ filters => cli_filters() }, {
        insta::assert_snapshot!(run(&["fmt", "--help"]));
    });
}

#[test]
fn snapshot_render_help() {
    insta::with_settings!({ filters => cli_filters() }, {
        insta::assert_snapshot!(run(&["render", "--help"]));
    });
}

#[test]
fn snapshot_inspect_help() {
    insta::with_settings!({ filters => cli_filters() }, {
        insta::assert_snapshot!(run(&["inspect", "--help"]));
    });
}

#[test]
fn snapshot_completions_help() {
    insta::with_settings!({ filters => cli_filters() }, {
        insta::assert_snapshot!(run(&["completions", "--help"]));
    });
}

#[test]
fn snapshot_explain_help() {
    insta::with_settings!({ filters => cli_filters() }, {
        insta::assert_snapshot!(run(&["explain", "--help"]));
    });
}

#[test]
fn snapshot_doctor_help() {
    insta::with_settings!({ filters => cli_filters() }, {
        insta::assert_snapshot!(run(&["doctor", "--help"]));
    });
}

#[test]
fn snapshot_init_help() {
    insta::with_settings!({ filters => cli_filters() }, {
        insta::assert_snapshot!(run(&["init", "--help"]));
    });
}
