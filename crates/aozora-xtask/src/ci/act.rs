//! `xtask ci act` — thin wrapper over [nektos/act][act] for replaying a
//! workflow job locally inside a Docker-emulated GitHub Actions runner.
//!
//! ## Why `act` and not "ssh into the runner"
//!
//! `act` is the de-facto local-Actions emulator: Go binary, MIT
//! licensed, ~30 k stars, actively maintained. It reads
//! `.github/workflows/*.yml` directly and spins each job inside a
//! Docker image that approximates the GitHub-hosted runner image.
//! Closer to a 1:1 reproduction than [`crate::ci::precheck`], at the
//! cost of pulling a heavy `catthehacker/ubuntu:act-22.04` (~1 GB)
//! image and giving up on the dev-image GHA cache.
//!
//! Use `precheck` for the fast loop; reach for `act` when:
//! - the workflow YAML itself is the suspect (matrix expansion,
//!   `if:` gating, action-version pinning, etc.),
//! - you want to test a `.github/actions/*` composite action without
//!   pushing,
//! - you want to verify a `secrets.*` reference resolves the way you
//!   expect.
//!
//! ## Why a wrapper at all
//!
//! Three things this binary does that a raw `act` invocation does not:
//! 1. Detect `act` on `PATH` and print a one-line install hint
//!    (`mise use -g github:nektos/act@latest`) when it's missing,
//!    instead of bouncing through `command not found`.
//! 2. Default to the right runner image
//!    (`-P ubuntu-latest=catthehacker/ubuntu:act-22.04`) so a
//!    first-time invocation doesn't trip the "what image should I
//!    use?" interactive prompt.
//! 3. Preserve the user's GitHub token from `gh auth token` so
//!    `actions/checkout` doesn't fail on private API rate limits.
//!
//! [act]: https://github.com/nektos/act

use std::process::Command;

use clap::Args;

const DEFAULT_RUNNER_IMAGE: &str = "catthehacker/ubuntu:act-22.04";

#[derive(Args)]
pub(crate) struct ActArgs {
    /// Workflow file basename (without `.yml`). Default: `ci`.
    #[arg(long, default_value = "ci")]
    workflow: String,

    /// Job name to run (must match `jobs.<name>` in the workflow).
    /// Required — running every job is heavy enough that it needs to
    /// be an explicit ask.
    #[arg(long, short = 'j')]
    job: String,

    /// Runner image. Override only if the default
    /// `catthehacker/ubuntu:act-22.04` doesn't fit (e.g. the larger
    /// `act-22.04-full` for actions that need extra system tools).
    #[arg(long, default_value = DEFAULT_RUNNER_IMAGE)]
    image: String,

    /// Pass through extra `act` arguments verbatim, e.g.
    /// `xtask ci act -j book -- --reuse --container-architecture linux/amd64`.
    #[arg(trailing_var_arg = true)]
    pass_through: Vec<String>,
}

pub(crate) fn run(args: &ActArgs) -> Result<(), String> {
    if which("act").is_none() {
        return Err(install_hint());
    }

    let mut cmd = Command::new("act");
    cmd.args(["-W", &format!(".github/workflows/{}.yml", args.workflow)])
        .args(["-j", &args.job])
        .args(["-P", &format!("ubuntu-latest={}", args.image)]);

    // Forward the user's gh-CLI token so checkout / API steps don't
    // hit anonymous rate limits. Best-effort — `gh auth token` exits
    // non-zero when the user isn't logged in, in which case we just
    // skip the secret and let act fall back to anonymous calls.
    if let Some(token) = gh_token() {
        cmd.args(["-s", &format!("GITHUB_TOKEN={token}")]);
    }

    cmd.args(&args.pass_through);

    let status = cmd
        .status()
        .map_err(|e| format!("failed to spawn `act`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("act exited with {status}"))
    }
}

fn which(prog: &str) -> Option<String> {
    let out = Command::new("which").arg(prog).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (!path.is_empty()).then_some(path)
}

fn gh_token() -> Option<String> {
    let out = Command::new("gh").args(["auth", "token"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let token = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (!token.is_empty()).then_some(token)
}

fn install_hint() -> String {
    "act is not on PATH.\n\
     \n\
     Install via mise (the workspace's chosen tool manager):\n\
     \n\
         mise use -g github:nektos/act@latest\n\
     \n\
     Or grab the binary directly: <https://github.com/nektos/act/releases>\n\
     \n\
     Then re-run `xtask ci act -j <job>`."
        .to_owned()
}
