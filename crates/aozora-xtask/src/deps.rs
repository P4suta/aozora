//! `xtask deps ...` — local-only dependency-follow-up tooling.
//!
//! Replaces the dependabot / renovate / GitHub Actions pattern with
//! a host-side systemd user timer that runs `just deps-check`
//! weekly. Why pure Rust and not a shell script:
//!
//! - the rest of the project is Rust 2024 — keeping tooling in the
//!   same language means one toolchain, one set of types, one set of
//!   error messages
//! - shell scripts add a parallel quoting / `set -euo pipefail`
//!   surface that is fundamentally harder to reason about
//! - portability: bash idioms break on Windows / non-bash shells; an
//!   `xtask` binary works wherever `cargo run` does
//!
//! Same precedent as the `samply` subcommand (see [`crate`] docs).
//!
//! ## Why on the host (not Docker)
//!
//! `systemctl --user` talks to the host's systemd user instance.
//! Inside the dev container there is no per-user systemd, so the
//! timer would never fire. Same reasoning as `xtask samply` —
//! tooling that touches the host kernel / init system runs on the
//! host, not in the container.
//!
//!
//!
//! ## Layout
//!
//! Two files end up in `$XDG_CONFIG_HOME/systemd/user/`:
//!
//! | File | Role |
//! |---|---|
//! | `aozora-deps-check.service` | one-shot unit that `cd`s into the repo and runs `just deps-check`, tee-ing output to `$XDG_STATE_HOME/aozora/deps-check.log` |
//! | `aozora-deps-check.timer`   | weekly trigger (Sun 03:30 local + 30 min jitter, `Persistent=true` so a missed run fires on next boot) |
//!
//! The unit is bound to the **specific repo checkout** that ran the
//! install (it bakes in `WorkingDirectory=…`). Re-running this
//! command from a different checkout simply overwrites the unit
//! files — they're keyed by name, not path.

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Args, Subcommand};

const SERVICE_NAME: &str = "aozora-deps-check";

#[derive(Args)]
pub(crate) struct DepsArgs {
    #[command(subcommand)]
    pub(crate) cmd: DepsCmd,
}

#[derive(Subcommand)]
pub(crate) enum DepsCmd {
    /// Install (or refresh) the systemd user timer that runs
    /// `just deps-check` weekly. Idempotent.
    InstallTimer,
    /// Remove the systemd user timer + service unit. The rolling
    /// log at `$XDG_STATE_HOME/aozora/deps-check.log` is preserved
    /// (delete manually if desired).
    UninstallTimer,
    /// Show the timer's current status, the next scheduled run,
    /// and the most recent journal entries.
    Status,
}

pub(crate) fn dispatch(args: &DepsArgs) -> Result<(), String> {
    match args.cmd {
        DepsCmd::InstallTimer => install_timer(),
        DepsCmd::UninstallTimer => uninstall_timer(),
        DepsCmd::Status => status(),
    }
}

// ---- subcommand implementations ------------------------------------

fn install_timer() -> Result<(), String> {
    let repo_root = locate_repo_root()?;
    let unit_dir = unit_dir()?;
    let state_dir = state_dir()?;
    let log_file = state_dir.join("deps-check.log");

    fs::create_dir_all(&unit_dir).map_err(|e| format!("create {}: {e}", unit_dir.display()))?;
    fs::create_dir_all(&state_dir).map_err(|e| format!("create {}: {e}", state_dir.display()))?;

    let service_unit = render_service_unit(&repo_root, &log_file);
    let timer_unit = render_timer_unit(&repo_root);

    let service_path = unit_dir.join(format!("{SERVICE_NAME}.service"));
    let timer_path = unit_dir.join(format!("{SERVICE_NAME}.timer"));

    fs::write(&service_path, service_unit)
        .map_err(|e| format!("write {}: {e}", service_path.display()))?;
    fs::write(&timer_path, timer_unit)
        .map_err(|e| format!("write {}: {e}", timer_path.display()))?;

    systemctl(["daemon-reload"])?;
    systemctl(["enable", "--now", &format!("{SERVICE_NAME}.timer")])?;

    println!("Installed:");
    println!("  {}", service_path.display());
    println!("  {}", timer_path.display());
    println!("Log: {}", log_file.display());
    println!();
    println!("Next run:");
    list_timers()?;
    println!();
    println!("Run once now (sanity-check):");
    println!("  systemctl --user start {SERVICE_NAME}.service");
    println!("Tail journal:");
    println!("  journalctl --user -u {SERVICE_NAME} -f");
    Ok(())
}

fn uninstall_timer() -> Result<(), String> {
    // `disable --now` is idempotent on systemd ≥ 220 — succeeds
    // whether the unit is enabled or not. We still suppress errors
    // so a partial install (e.g. unit files present, never enabled)
    // can be cleaned up.
    drop(systemctl([
        "disable",
        "--now",
        &format!("{SERVICE_NAME}.timer"),
    ]));

    let unit_dir = unit_dir()?;
    let service_path = unit_dir.join(format!("{SERVICE_NAME}.service"));
    let timer_path = unit_dir.join(format!("{SERVICE_NAME}.timer"));

    for path in [&timer_path, &service_path] {
        match fs::remove_file(path) {
            Ok(()) => println!("removed {}", path.display()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                println!("(already absent) {}", path.display());
            }
            Err(e) => return Err(format!("remove {}: {e}", path.display())),
        }
    }

    systemctl(["daemon-reload"])?;
    let log_file = state_dir()?.join("deps-check.log");
    println!();
    println!(
        "(log file at {} preserved — delete manually if desired)",
        log_file.display()
    );
    Ok(())
}

fn status() -> Result<(), String> {
    if !unit_installed()? {
        return Err(
            "Timer not installed. Run: cargo run -p aozora-xtask -- deps install-timer".to_owned(),
        );
    }
    // `status` exits non-zero whenever a unit is inactive (which is
    // normal for a oneshot service between firings) — capture
    // stdout regardless and print it.
    print_systemctl(["status", &format!("{SERVICE_NAME}.timer"), "--no-pager"]);
    println!();
    println!("Recent runs:");
    print_systemctl_raw(
        "journalctl",
        ["--user", "-u", SERVICE_NAME, "-n", "30", "--no-pager"],
    );
    Ok(())
}

// ---- helpers -------------------------------------------------------

fn locate_repo_root() -> Result<PathBuf, String> {
    // Walk up from CWD until we find a Cargo.toml that contains
    // `[workspace]`. Mirrors how `cargo` discovers the workspace
    // root and avoids hard-coding the path / requiring an env var.
    let cwd = env::current_dir().map_err(|e| format!("cwd: {e}"))?;
    let mut here: &Path = &cwd;
    loop {
        let cargo_toml = here.join("Cargo.toml");
        if cargo_toml.exists() {
            let body = fs::read_to_string(&cargo_toml)
                .map_err(|e| format!("read {}: {e}", cargo_toml.display()))?;
            if body.contains("[workspace]") {
                return Ok(here.to_path_buf());
            }
        }
        match here.parent() {
            Some(parent) => here = parent,
            None => {
                return Err(
                    "could not find workspace root (no Cargo.toml with [workspace] above CWD)"
                        .into(),
                );
            }
        }
    }
}

fn unit_dir() -> Result<PathBuf, String> {
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok_or("neither XDG_CONFIG_HOME nor HOME is set")?;
    Ok(base.join("systemd").join("user"))
}

fn state_dir() -> Result<PathBuf, String> {
    let base = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .ok_or("neither XDG_STATE_HOME nor HOME is set")?;
    Ok(base.join("aozora"))
}

fn render_service_unit(repo_root: &Path, log_file: &Path) -> String {
    // Hard-fail (`ConditionPathExists`) rather than half-run if the
    // repo or Docker disappears — the health-check is meaningless
    // without the dev container `just deps-check` invokes.
    format!(
        "\
[Unit]
Description=Weekly aozora workspace dependency-health check (just deps-check)
Documentation=file://{repo}/CONTRIBUTING.md
ConditionPathExists={repo}/Justfile
ConditionPathExists=/var/run/docker.sock

[Service]
Type=oneshot
WorkingDirectory={repo}
Environment=PATH=/usr/local/bin:/usr/bin:/bin
# Tee output to a rolling log so 'just deps-status' (and the user's
# eyeball) can see the most recent run; `journalctl --user -u {svc}`
# is the structured alternative.
ExecStart=/bin/bash -c 'just deps-check 2>&1 | tee -a \"{log}\"'
Nice=10
IOSchedulingClass=idle
",
        repo = repo_root.display(),
        log = log_file.display(),
        svc = SERVICE_NAME,
    )
}

fn render_timer_unit(repo_root: &Path) -> String {
    format!(
        "\
[Unit]
Description=Run aozora deps-check weekly (Sunday 03:30 local)
Documentation=file://{repo}/CONTRIBUTING.md

[Timer]
# Sunday at 03:30 local time, with Persistent=true so a missed run
# (laptop asleep) fires on next boot. RandomizedDelaySec spreads
# multiple machines on the same LAN so we don't hit crates.io in
# lock-step. Same cadence as the user-level 'mise upgrade --bump'
# timer (dotfiles) — keeps tooling and deps refresh on the same
# weekly rhythm.
OnCalendar=Sun *-*-* 03:30:00
Persistent=true
RandomizedDelaySec=30m

[Install]
WantedBy=timers.target
",
        repo = repo_root.display(),
    )
}

fn systemctl<I, S>(args: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = Command::new("systemctl");
    cmd.arg("--user");
    for a in args {
        cmd.arg(a);
    }
    let status = cmd
        .status()
        .map_err(|e| format!("spawn systemctl --user: {e}"))?;
    if !status.success() {
        return Err(format!("systemctl --user exited with status {status}"));
    }
    Ok(())
}

fn print_systemctl<const N: usize>(args: [&str; N]) {
    print_systemctl_raw("systemctl", {
        let mut full: Vec<&str> = vec!["--user"];
        full.extend(args);
        full
    });
}

fn print_systemctl_raw<I, S>(bin: &str, args: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = Command::new(bin);
    for a in args {
        cmd.arg(a);
    }
    // Inherit stdio so the user sees the systemd output verbatim.
    drop(cmd.status());
}

fn list_timers() -> Result<(), String> {
    let mut cmd = Command::new("systemctl");
    cmd.args([
        "--user",
        "list-timers",
        &format!("{SERVICE_NAME}.timer"),
        "--no-pager",
    ]);
    cmd.status()
        .map_err(|e| format!("spawn systemctl list-timers: {e}"))?;
    Ok(())
}

fn unit_installed() -> Result<bool, String> {
    let path = unit_dir()?.join(format!("{SERVICE_NAME}.timer"));
    Ok(path.exists())
}
