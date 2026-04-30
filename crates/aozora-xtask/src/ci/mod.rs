//! `xtask ci …` — local-side instrumentation around the GitHub Actions
//! pipeline.
//!
//! Three subcommands sit under this module:
//!
//! - [`profile`] — pull a finished workflow run from the GitHub API and
//!   rank its jobs / steps by wall time. Data-driven optimisation.
//! - [`precheck`] — run every CI job locally through the same `just`
//!   recipes the dev-image-based CI workflow uses, and report a per-job
//!   wall time table in the same shape as `profile`. Push-time
//!   confidence without waiting on Actions.
//! - [`act`] — thin wrapper over [`nektos/act`](https://github.com/nektos/act),
//!   the upstream tool that re-runs `.github/workflows/*.yml` inside
//!   Docker. Closer to a 1:1 GitHub-runner emulation than `precheck`,
//!   at the cost of pulling a heavyweight Docker image and giving up
//!   on the dev-image cache. Use when `precheck` agrees but you still
//!   want to validate the workflow YAML itself.

use clap::{Args, Subcommand};

mod act;
mod precheck;
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
    /// Run every CI job locally (`docker compose run dev just <target>`)
    /// and emit a per-job wall-time table in the same shape as
    /// `profile`. Push-time confidence loop.
    Precheck(precheck::PrecheckArgs),
    /// Drive `nektos/act` to re-run a workflow job inside the GitHub
    /// runner image. Heavier than `precheck`; reach for it when the
    /// concern is the workflow YAML itself rather than the underlying
    /// command.
    Act(act::ActArgs),
}

pub(crate) fn run(args: &CiArgs) -> Result<(), String> {
    match &args.cmd {
        CiCmd::Profile(p) => profile::run(p),
        CiCmd::Precheck(p) => precheck::run(p),
        CiCmd::Act(p) => act::run(p),
    }
}
