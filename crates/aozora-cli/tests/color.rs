//! End-to-end colour policy and anti-hang guard tests for the `aozora`
//! binary.
//!
//! Spawns the *built* binary (via the `CARGO_BIN_EXE_aozora` env var Cargo
//! injects), so we exercise the real `--color` plumbing: the global flag, the
//! process-wide miette hook it installs, and miette's own `NO_COLOR` /
//! `CLICOLOR` / `CLICOLOR_FORCE` detection on the `auto` path.
//!
//! Colour is asserted by the presence of an ESC byte (`0x1b`) in the raw
//! output — the same technique as `aozora-fmt`'s `--diff --color` test.
//!
//! Every spawn clears the three colour-control env vars first, so the ambient
//! test environment cannot perturb an auto-detection assertion; each test
//! then sets only the vars it means to exercise. Pure stdlib, no extra
//! dev-dep — matching the `smoke.rs` house style.
//!
//! Note on the interactive-TTY guard: the test harness's stdin is never a
//! terminal, so [`aozora check`] with no input under `cargo test` takes the
//! *piped* branch (empty document, exit 0), which the regression test below
//! pins. The guard's actual hit path — a bare TTY — is covered by the
//! `write_stdin_hint` unit test in `src/input.rs`, plus the manual check
//! documented in the PR.

use std::io::Write;
use std::process::{Command, ExitStatus, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_aozora");

/// Input that provokes at least one diagnostic — a literal PUA sentinel
/// (U+E001) trips `SourceContainsPua` — so `check --diagnostic-format human`
/// actually renders a miette report whose colour we can inspect.
const DIRTY: &str = "abc\u{E001}def";

struct Output {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// Spawn `aozora <args>` with `env` applied (over a cleared colour
/// environment) and `stdin` optionally piped in. Returns the raw bytes so
/// ESC-byte assertions see exactly what the terminal would.
fn run(args: &[&str], env: &[(&str, &str)], stdin: Option<&str>) -> Output {
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR")
        .env_remove("CLICOLOR_FORCE")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }
    for (key, value) in env {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().expect("spawn aozora");
    if let Some(s) = stdin {
        child
            .stdin
            .as_mut()
            .expect("piped stdin")
            .write_all(s.as_bytes())
            .expect("write stdin");
    }
    let out = child.wait_with_output().expect("wait for aozora");
    Output {
        status: out.status,
        stdout: out.stdout,
        stderr: out.stderr,
    }
}

/// `aozora check` in the human diagnostic format (so miette renders) under
/// the given `--color` value and `env`, fed dirty input on stdin.
fn check_human(color: &str, env: &[(&str, &str)]) -> Output {
    run(
        &["check", "--diagnostic-format", "human", "--color", color],
        env,
        Some(DIRTY),
    )
}

/// True if `bytes` contains an ANSI escape introducer (ESC, `0x1b`).
fn has_ansi(bytes: &[u8]) -> bool {
    bytes.contains(&0x1b)
}

// ---------------------------------------------------------------------
// Explicit --color wins unconditionally
// ---------------------------------------------------------------------

#[test]
fn color_always_emits_ansi() {
    let out = check_human("always", &[]);
    assert!(
        has_ansi(&out.stderr),
        "--color always must emit ANSI, got: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn color_never_emits_no_ansi() {
    let out = check_human("never", &[]);
    assert!(
        !has_ansi(&out.stderr),
        "--color never must not emit ANSI, got: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn color_always_beats_no_color_env() {
    let out = check_human("always", &[("NO_COLOR", "1")]);
    assert!(
        has_ansi(&out.stderr),
        "--color always must override NO_COLOR"
    );
}

#[test]
fn color_never_beats_clicolor_force_env() {
    let out = check_human("never", &[("CLICOLOR_FORCE", "1")]);
    assert!(
        !has_ansi(&out.stderr),
        "--color never must override CLICOLOR_FORCE"
    );
}

// ---------------------------------------------------------------------
// --color auto defers to miette's env + TTY detection
// ---------------------------------------------------------------------

#[test]
fn color_auto_on_a_pipe_is_monochrome() {
    // stderr is a pipe, not a TTY, and no CLICOLOR_FORCE → no colour.
    let out = check_human("auto", &[]);
    assert!(
        !has_ansi(&out.stderr),
        "auto on a piped stderr must stay monochrome"
    );
}

#[test]
fn color_auto_respects_no_color() {
    let out = check_human("auto", &[("NO_COLOR", "1")]);
    assert!(!has_ansi(&out.stderr), "NO_COLOR must keep auto monochrome");
}

#[test]
fn color_auto_respects_clicolor_force() {
    let out = check_human("auto", &[("CLICOLOR_FORCE", "1")]);
    assert!(
        has_ansi(&out.stderr),
        "CLICOLOR_FORCE must force auto colour even on a pipe"
    );
}

// ---------------------------------------------------------------------
// `aozora kinds` is monochrome by construction (comfy-table built
// without its tty feature) regardless of flag or env.
// ---------------------------------------------------------------------

#[test]
fn kinds_is_always_monochrome() {
    let out = run(
        &["kinds", "--color", "always"],
        &[("CLICOLOR_FORCE", "1")],
        None,
    );
    assert!(
        out.status.success(),
        "kinds must succeed: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!has_ansi(&out.stdout), "kinds stdout must never emit ANSI");
    assert!(!has_ansi(&out.stderr), "kinds stderr must never emit ANSI");
}

// ---------------------------------------------------------------------
// Anti-hang guard: piped (non-TTY) stdin is NOT interactive, so the
// guard stands aside and empty input parses to an empty doc (exit 0).
// ---------------------------------------------------------------------

#[test]
fn empty_piped_stdin_check_exits_zero() {
    let out = run(&["check"], &[], Some(""));
    assert!(
        out.status.success(),
        "empty piped stdin must not trip the guard; expected exit 0, got {:?}",
        out.status
    );
}

#[test]
fn piped_dirty_stdin_check_still_runs() {
    // A pipe with content is likewise never interactive: the guard passes
    // and the document is processed normally (exit 0 without --strict).
    let out = run(&["check"], &[], Some(DIRTY));
    assert!(
        out.status.success(),
        "piped dirty input must process normally, got {:?}",
        out.status
    );
}
