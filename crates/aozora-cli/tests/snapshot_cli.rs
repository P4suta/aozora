//! Snapshots of CLI help, version, and subcommand output.

use std::process::Command;

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
    let stdout = normalize(&String::from_utf8(output.stdout).expect("CLI stdout is UTF-8"));
    let stderr = normalize(&String::from_utf8(output.stderr).expect("CLI stderr is UTF-8"));
    if stderr.is_empty() {
        stdout
    } else {
        format!("{stdout}\n----- STDERR -----\n{stderr}")
    }
}

fn normalize(text: &str) -> String {
    let final_newline = text.ends_with('\n');
    let mut normalized = text
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    if final_newline {
        normalized.push('\n');
    }
    normalized
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
fn snapshot_pandoc_help() {
    insta::with_settings!({ filters => cli_filters() }, {
        insta::assert_snapshot!(run(&["pandoc", "--help"]));
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

#[test]
fn snapshot_repl_help() {
    insta::with_settings!({ filters => cli_filters() }, {
        insta::assert_snapshot!(run(&["repl", "--help"]));
    });
}

#[test]
fn snapshot_tui_help() {
    insta::with_settings!({ filters => cli_filters() }, {
        insta::assert_snapshot!(run(&["tui", "--help"]));
    });
}
