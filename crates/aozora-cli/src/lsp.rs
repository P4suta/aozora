//! `aozora lsp` — exec-delegate to the `aozora-lsp` language-server daemon.
//!
//! The CLI bundles no LSP machinery of its own — no tokio, no tower-lsp. This
//! subcommand is a thin git-`<x>`-style shim: it locates the `aozora-lsp`
//! binary and hands the whole process over to it, forwarding every argument
//! (e.g. `--stdio`) untouched. On Unix it `exec`s, replacing this process so
//! no wrapper lingers for the editor session's lifetime; elsewhere it spawns
//! the daemon and forwards its exit status.
//!
//! The daemon is resolved on `PATH` first, then next to the running executable
//! (a release tarball keeps the two binaries side by side even when that
//! directory is not on `PATH`). When it cannot be found the shim prints the
//! same actionable shape as the `pandoc`-missing message and exits 2 — a usage
//! error, not the generic failure (1), so an editor / script can tell "the
//! server is not installed" apart from "the server crashed".

use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode};

use anyhow::{Context, Result};
use clap::Parser;

use crate::which;

/// The daemon binary this subcommand delegates to. Bare name; the platform's
/// executable extension (`.exe` on Windows) is appended by [`which`] during the
/// candidate search.
const LSP_BIN: &str = "aozora-lsp";

/// The actionable hint printed when `aozora-lsp` cannot be located — the same
/// "here is what to install and why" shape as the `pandoc`-missing error, minus
/// the leading `aozora:` program prefix the caller adds.
const MISSING_LSP: &str = "`aozora-lsp` was not found on PATH or next to this binary; \
     install the aozora language server (it ships alongside `aozora` in the release \
     tarball) so `aozora lsp` can delegate to it";

/// `aozora lsp [ARGS…]` — every argument is forwarded verbatim to the
/// `aozora-lsp` daemon; this shim parses none of its own.
#[derive(Debug, Parser)]
#[command(
    // A pure pass-through: `--help` / `--version` belong to the daemon, not to
    // this shim, so `aozora lsp --help` shows the *server's* help. (The shim's
    // own summary is still reachable via `aozora help lsp`.) Without disabling
    // them clap would intercept the flags before the trailing positional.
    disable_help_flag = true,
    disable_version_flag = true,
    after_long_help = "Examples:
  aozora lsp --stdio    # what an editor spawns (LSP over stdio)
  aozora lsp --help     # forwarded: prints aozora-lsp's own help

Every argument after `lsp` is passed straight to the aozora-lsp daemon."
)]
pub(crate) struct LspArgs {
    /// Arguments forwarded verbatim to the `aozora-lsp` daemon (e.g.
    /// `--stdio`). Everything after `lsp` is passed through untouched; this
    /// shim adds no flags of its own.
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "ARGS"
    )]
    args: Vec<OsString>,
}

/// Locate `aozora-lsp` and hand off to it, forwarding `args.args` verbatim.
/// Missing daemon → the actionable message on stderr and exit 2.
pub(crate) fn run(args: &LspArgs) -> Result<ExitCode> {
    let Some(bin) = locate(env::var_os("PATH").as_deref(), current_exe_dir().as_deref()) else {
        let _drop = writeln!(io::stderr(), "aozora: {MISSING_LSP}");
        return Ok(ExitCode::from(2));
    };
    delegate(&bin, &args.args)
}

/// Resolve the daemon: on `PATH` first, then beside the running executable.
/// Pure over its inputs — the raw `PATH` value and the executable's directory —
/// so it is unit-testable without mutating the process environment or relying
/// on a real install.
fn locate(path: Option<&OsStr>, exe_dir: Option<&Path>) -> Option<PathBuf> {
    path.and_then(|paths| which::find_on_path(paths, LSP_BIN))
        .or_else(|| exe_dir.and_then(|dir| which::find_in_dir(dir, LSP_BIN)))
}

/// The directory holding the running `aozora` executable, if it can be
/// resolved — the "next to this binary" fallback's search root.
fn current_exe_dir() -> Option<PathBuf> {
    env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

/// Hand the process over to `bin` with `args`. On Unix this `exec`s, replacing
/// the current process image so no `aozora` wrapper lingers for the LSP
/// session's lifetime (the git-`<x>` delegation model); a return means `exec`
/// itself failed (the binary vanished between locate and exec, say).
#[cfg(unix)]
fn delegate(bin: &Path, args: &[OsString]) -> Result<ExitCode> {
    use std::os::unix::process::CommandExt as _;
    // `exec` only ever returns on failure; on success control never comes back.
    let err = Process::new(bin).args(args).exec();
    Err(err).with_context(|| format!("failed to exec `{}`", bin.display()))
}

/// The non-Unix hand-off: no `exec`, so spawn the daemon, wait, and forward its
/// exit status (a signal death maps to the generic failure code).
#[cfg(not(unix))]
fn delegate(bin: &Path, args: &[OsString]) -> Result<ExitCode> {
    let status = Process::new(bin)
        .args(args)
        .status()
        .with_context(|| format!("failed to run `{}`", bin.display()))?;
    Ok(status.code().map_or(ExitCode::FAILURE, |code| {
        ExitCode::from(u8::try_from(code).unwrap_or(1))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_message_is_actionable() {
        // The hint must name the binary, the fact that both PATH and the
        // sibling dir were searched, and how to fix it — the same "what to
        // install and why" shape as the pandoc-missing error.
        assert!(MISSING_LSP.contains("aozora-lsp"), "names the binary");
        assert!(MISSING_LSP.contains("PATH"), "mentions the PATH search");
        assert!(
            MISSING_LSP.contains("next to this binary"),
            "mentions the sibling fallback"
        );
        assert!(MISSING_LSP.contains("install"), "tells the user to install");
    }

    #[cfg(unix)]
    #[test]
    fn locate_prefers_path_over_the_sibling_dir() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt as _;

        let path_dir = tempfile::TempDir::new().expect("path dir");
        let exe_dir = tempfile::TempDir::new().expect("exe dir");
        // An executable daemon in BOTH the PATH dir and the sibling dir.
        for dir in [path_dir.path(), exe_dir.path()] {
            let bin = dir.join(LSP_BIN);
            fs::write(&bin, b"#!/bin/sh\n").expect("write stub");
            fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).expect("chmod +x");
        }
        let path = env::join_paths([path_dir.path()]).expect("join PATH");
        assert_eq!(
            locate(Some(path.as_os_str()), Some(exe_dir.path())).as_deref(),
            Some(path_dir.path().join(LSP_BIN).as_path()),
            "PATH wins over the sibling directory"
        );
    }

    #[cfg(unix)]
    #[test]
    fn locate_falls_back_to_the_sibling_dir_when_not_on_path() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt as _;

        let empty_path = tempfile::TempDir::new().expect("empty path dir");
        let exe_dir = tempfile::TempDir::new().expect("exe dir");
        let sibling = exe_dir.path().join(LSP_BIN);
        fs::write(&sibling, b"#!/bin/sh\n").expect("write stub");
        fs::set_permissions(&sibling, fs::Permissions::from_mode(0o755)).expect("chmod +x");

        let path = env::join_paths([empty_path.path()]).expect("join PATH");
        assert_eq!(
            locate(Some(path.as_os_str()), Some(exe_dir.path())).as_deref(),
            Some(sibling.as_path()),
            "the sibling daemon is found when PATH has none"
        );
    }

    #[test]
    fn locate_is_none_when_the_daemon_is_nowhere() {
        // Neither an empty PATH (one empty entry, filtered out) nor an absent
        // sibling dir yields a daemon — the exit-2 "not installed" path.
        let empty = OsString::new();
        assert_eq!(
            locate(Some(empty.as_os_str()), None),
            None,
            "empty PATH + no exe dir -> None"
        );
    }
}
