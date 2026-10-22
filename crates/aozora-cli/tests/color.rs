//! End-to-end colour policy and anti-hang guard tests for the `aozora` binary.

use std::io::Write;
use std::process::{Command, ExitStatus, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_aozora");

const DIRTY: &str = "abc\u{E001}def";

struct Output {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run(args: &[&str], env: &[(&str, &str)], stdin: Option<&str>) -> Output {
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .env("AOZORA_LANG", "en")
        .env_remove("LANG")
        .env_remove("LC_ALL")
        .env_remove("NO_COLOR")
        .env_remove("CLICOLOR")
        .env_remove("CLICOLOR_FORCE")
        .env_remove("FORCE_COLOR")
        .env("XDG_CONFIG_HOME", env!("CARGO_TARGET_TMPDIR"))
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

fn check_human(color: &str, env: &[(&str, &str)]) -> Output {
    run(
        &["check", "--format", "human", "--color", color],
        env,
        Some(DIRTY),
    )
}

fn has_ansi(bytes: &[u8]) -> bool {
    bytes.contains(&0x1b)
}

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

#[test]
fn color_auto_on_a_pipe_is_monochrome() {
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

#[test]
fn kinds_is_always_monochrome() {
    let out = run(
        &["spec", "kinds", "--color", "always"],
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

#[derive(Clone, Copy, Debug)]
enum Machine {
    Stdout,
    Stderr,
}

impl Machine {
    fn of(self, out: &Output) -> &[u8] {
        match self {
            Self::Stdout => &out.stdout,
            Self::Stderr => &out.stderr,
        }
    }
}

const MACHINE_CONTRACTS: &[(&[&str], Option<&str>, Machine)] = &[
    (&["inspect", "nodes"], Some(DIRTY), Machine::Stdout),
    (
        &["spec", "kinds", "--format", "json"],
        None,
        Machine::Stdout,
    ),
    (&["explain", "ruby"], None, Machine::Stdout),
    (&["check", "--format", "json"], Some(DIRTY), Machine::Stderr),
    (
        &["check", "--format", "short"],
        Some(DIRTY),
        Machine::Stderr,
    ),
];

const COLOUR_ENVIRONMENTS: &[(&str, &str)] = &[
    ("NO_COLOR", "1"),
    ("CLICOLOR", "0"),
    ("CLICOLOR_FORCE", "1"),
    ("FORCE_COLOR", "1"),
];

#[test]
fn the_colour_axis_cannot_touch_machine_output() {
    for (args, stdin, stream) in MACHINE_CONTRACTS {
        let baseline = run(args, &[], *stdin);
        assert!(
            baseline.status.success(),
            "baseline `{args:?}` must exit 0: {:?}",
            String::from_utf8_lossy(&baseline.stderr)
        );
        let expected = stream.of(&baseline);
        assert!(
            !expected.is_empty(),
            "`{args:?}` must carry its machine output on {stream:?}, else the \
             byte-identity assertions below pin nothing"
        );
        assert!(
            !has_ansi(expected),
            "`{args:?}` machine output must never be coloured"
        );

        for (key, value) in COLOUR_ENVIRONMENTS {
            let out = run(args, &[(key, value)], *stdin);
            assert!(
                out.status.success(),
                "{key}={value} must not break `{args:?}`; got {:?}: {:?}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(
                stream.of(&out),
                expected,
                "{key}={value} must leave `{args:?}` {stream:?} byte-identical"
            );
        }

        for choice in ["auto", "always", "never"] {
            let mut with_flag = args.to_vec();
            with_flag.extend_from_slice(&["--color", choice]);
            let out = run(&with_flag, &[], *stdin);
            assert!(
                out.status.success(),
                "--color {choice} must not break `{args:?}`: {:?}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert_eq!(
                stream.of(&out),
                expected,
                "--color {choice} must leave `{args:?}` {stream:?} byte-identical"
            );
        }
    }
}

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
    let out = run(&["check"], &[], Some(DIRTY));
    assert!(
        out.status.success(),
        "piped dirty input must process normally, got {:?}",
        out.status
    );
}
