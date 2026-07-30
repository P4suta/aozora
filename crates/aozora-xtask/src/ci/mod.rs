//! `xtask ci …` — local-side instrumentation around the GitHub Actions
//! pipeline.
//!
//! The profile subcommand pulls a finished workflow run from the GitHub API
//! and ranks its jobs and steps by wall time.

use clap::{Args, Subcommand};

mod profile;

#[derive(Args)]
pub(crate) struct CiArgs {
    #[command(subcommand)]
    cmd: CiCmd,
}

#[derive(Subcommand)]
enum CiCmd {
    /// Profile the per-job + per-step wall time of a workflow run via
    /// the GitHub API (uses the local `gh` CLI).
    Profile(profile::ProfileArgs),
}

pub(crate) fn run(args: &CiArgs) -> Result<(), String> {
    match &args.cmd {
        CiCmd::Profile(p) => profile::run(p),
    }
}
