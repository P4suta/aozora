//! `xtask ci precheck` — run every CI job locally + print a per-job
//! wall-time table in the same shape as `ci profile`.
//!
//! ## Why this exists
//!
//! The CI round-trip is expensive (push → queue → 5–45 min → fetch
//! logs → diagnose → push). The same recipes the CI workflow runs
//! (`just lint`, `just build`, `just test`, …) are available locally,
//! and lefthook already wires fmt+clippy+typos into pre-commit. The
//! gap is "run the *whole* CI surface end-to-end at once with a
//! single command, get a table back, see where the time went".
//!
//! `xtask ci precheck` fills that gap: one command, one table, push
//! when it's green.
//!
//! ## What it runs
//!
//! Each entry in [`JOBS`] maps a CI job name to a shell-equivalent
//! recipe. Most go through `just <target>` (the same path the dev-
//! image-based CI jobs use); the three native-runner jobs (`smoke-ffi`,
//! `wasm-build`, `python-wheel`) shell out to their host commands so
//! the parity with `ci.yml` is preserved.
//!
//! Skipped on purpose:
//! - `msrv`: needs a different rustc pin than the dev image carries;
//!   would force a second toolchain install. Run on demand with
//!   `cargo +1.95.0 check --workspace --all-targets`.
//! - `commitlint`: PR-only.

use std::cmp::Reverse;
use std::process::Command;
use std::time::Instant;

use clap::Args;

#[derive(Args)]
pub(crate) struct PrecheckArgs {
    /// Comma-separated list of jobs to run. Default: every gated job.
    #[arg(long)]
    jobs: Option<String>,

    /// Stop on the first failure rather than continuing to the next.
    /// Off by default — running every job + reading the timing table is
    /// usually more useful than bailing on job 2 of 8.
    #[arg(long)]
    fail_fast: bool,

    /// Show available jobs and exit.
    #[arg(long)]
    list: bool,
}

struct JobSpec {
    /// CI job name (must match `.github/workflows/ci.yml`).
    name: &'static str,
    /// Human description shown in `--list`.
    summary: &'static str,
    /// Argv to spawn (first element is the program).
    argv: &'static [&'static str],
}

/// Mapping of CI jobs to their local-equivalent commands.
///
/// Adding a job here is the right reflex when adding a new CI gate
/// — keeps `precheck` honest with the workflow.
const JOBS: &[JobSpec] = &[
    JobSpec {
        name: "lint",
        summary: "fmt + clippy + typos + strict-code",
        argv: &["just", "lint"],
    },
    JobSpec {
        name: "build-and-test",
        summary: "cargo build --workspace --all-targets + nextest",
        argv: &["bash", "-c", "just build && just test"],
    },
    JobSpec {
        name: "coverage",
        summary: "cargo llvm-cov nextest with region floor",
        argv: &["just", "coverage"],
    },
    JobSpec {
        name: "audit-deny",
        summary: "cargo-deny licenses + advisories + bans",
        argv: &["just", "deny"],
    },
    JobSpec {
        name: "audit-audit",
        summary: "cargo-audit RustSec advisory db scan",
        argv: &["just", "audit"],
    },
    JobSpec {
        name: "audit-udeps",
        summary: "cargo-udeps unused-dependency scan (nightly)",
        argv: &["just", "udeps"],
    },
    JobSpec {
        name: "smoke-ffi",
        summary: "C ABI smoke test (host)",
        argv: &["just", "smoke-ffi"],
    },
    JobSpec {
        name: "book",
        summary: "mdbook build + lychee link verification",
        argv: &["bash", "-c", "just book-build && just book-linkcheck"],
    },
];

pub(crate) fn run(args: &PrecheckArgs) -> Result<(), String> {
    if args.list {
        print_jobs();
        return Ok(());
    }

    let selected: Vec<&JobSpec> = match args.jobs.as_deref() {
        None | Some("all") => JOBS.iter().collect(),
        Some(csv) => {
            let names: Vec<&str> = csv.split(',').map(str::trim).collect();
            let mut out = Vec::with_capacity(names.len());
            for name in names {
                let job = JOBS.iter().find(|j| j.name == name).ok_or_else(|| {
                    format!(
                        "unknown job '{name}'. Try: {}",
                        JOBS.iter().map(|j| j.name).collect::<Vec<_>>().join(",")
                    )
                })?;
                out.push(job);
            }
            out
        }
    };

    // CI wall times are reported in seconds; sub-second precision
    // would not match what `ci profile` shows from the GitHub API
    // anyway, so keep the locally-measured durations on the same
    // i64-second scale for parity.
    let mut results: Vec<(&str, i64, bool)> = Vec::with_capacity(selected.len());
    let mut overall_ok = true;
    for job in &selected {
        println!();
        println!("=== {} ===", job.name);
        let started = Instant::now();
        let status = Command::new(job.argv[0]).args(&job.argv[1..]).status();
        // `as_secs()` returns u64 for non-negative `Duration`s; the
        // realistic upper bound for a single CI step is hours, well
        // within i64. `i64::try_from` makes the bound explicit.
        let dur = i64::try_from(started.elapsed().as_secs()).unwrap_or(i64::MAX);
        let ok = matches!(status, Ok(s) if s.success());
        results.push((job.name, dur, ok));
        overall_ok &= ok;
        if !ok && args.fail_fast {
            println!("\n[fail-fast] {} failed at {dur}s; stopping.", job.name);
            break;
        }
    }

    println!();
    println!("ci precheck summary");
    println!();
    println!("{:<20} {:<8} {:>10}", "JOB", "RESULT", "WALL (s)");
    println!("{}", "-".repeat(45));
    let mut sorted = results.clone();
    sorted.sort_by_key(|t| Reverse(t.1));
    for (name, dur, ok) in &sorted {
        let mark = if *ok { "ok" } else { "FAIL" };
        println!("{name:<20} {mark:<8} {dur:>10}");
    }
    let total: i64 = results.iter().map(|(_, d, _)| d).sum();
    println!("{}", "-".repeat(45));
    println!(
        "{:<20} {:<8} {total:>10}",
        "TOTAL",
        if overall_ok { "ok" } else { "FAIL" }
    );

    if overall_ok {
        Ok(())
    } else {
        Err("one or more precheck jobs failed; do not push".to_owned())
    }
}

fn print_jobs() {
    println!("{:<18} DESCRIPTION", "JOB");
    println!("{}", "-".repeat(70));
    for j in JOBS {
        println!("{:<18} {}", j.name, j.summary);
    }
    println!();
    println!("Use:");
    println!("  xtask ci precheck                         # run all jobs");
    println!("  xtask ci precheck --jobs lint,book        # run a subset");
    println!("  xtask ci precheck --jobs all --fail-fast  # stop on first failure");
}
