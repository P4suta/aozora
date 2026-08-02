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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::iter::once;
    use std::path::Path;

    #[test]
    fn mutation_workflows_install_the_configured_test_runner() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let config = fs::read_to_string(root.join("mutants.toml")).expect("mutants config");
        let config: toml::Value = toml::from_str(&config).expect("parse mutants config");
        assert_eq!(
            config.get("test_tool").and_then(toml::Value::as_str),
            Some("nextest")
        );

        let job_header =
            regex::Regex::new(r"(?m)^  [A-Za-z0-9_-]+:\r?$\n").expect("workflow job pattern");
        let mut mutation_job_found = false;
        for entry in fs::read_dir(root.join(".github/workflows")).expect("workflow directory") {
            let path = entry.expect("workflow entry").path();
            if !matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("yml" | "yaml")
            ) {
                continue;
            }
            let workflow = fs::read_to_string(&path).expect("workflow source");
            let starts = job_header
                .find_iter(&workflow)
                .map(|matched| matched.start())
                .chain(once(workflow.len()))
                .collect::<Vec<_>>();
            for bounds in starts.windows(2) {
                let job = &workflow[bounds[0]..bounds[1]];
                if !job.contains("just mutants") {
                    continue;
                }
                mutation_job_found = true;
                assert!(
                    job.contains("cargo:cargo-mutants") && job.contains("cargo:cargo-nextest"),
                    "{} runs mutation tests without both required tools",
                    path.display()
                );
            }
        }
        assert!(mutation_job_found);
    }
}
