//! `xtask ci profile` — data-driven CI wall-clock profiler.
//!
//! Rust CI on GitHub Actions has well-known long-tail behaviour:
//! Docker layer rebuilds, `cargo install` of dev tooling, and external
//! checks like `lychee` can each silently consume tens of minutes
//! without the workflow log surfacing where the time went. The
//! `xtask ci profile` subcommand pulls the per-job and per-step wall
//! times of a given workflow run and ranks them, so optimisation
//! decisions are driven by the actual numbers rather than guesses.
//!
//! ## Usage
//!
//! ```text
//! # Latest ci.yml run on main
//! cargo run -p aozora-xtask --release -- ci profile
//!
//! # A specific run
//! cargo run -p aozora-xtask --release -- ci profile --run-id 25144476719
//!
//! # A different workflow
//! cargo run -p aozora-xtask --release -- ci profile --workflow docs
//!
//! # Multiple recent runs (median per job)
//! cargo run -p aozora-xtask --release -- ci profile --limit 5
//! ```
//!
//! ## Wire shape
//!
//! Talks to GitHub via the locally-installed `gh` CLI (the same auth
//! every other xtask path already relies on — see CLAUDE.md note that
//! `gh` is logged in via SSH as `P4suta`). No `reqwest`/`octocrab`
//! dependency, no PAT plumbing, no rate-limit retry loop to maintain.
//!
//! Two `gh api` calls per profile:
//!   - `repos/{owner}/{repo}/actions/runs?branch=…&per_page=…`
//!   - `repos/{owner}/{repo}/actions/runs/{id}/jobs?per_page=100`
//!
//! Both go through `gh api --paginate`, so multi-page job lists are
//! transparently merged.
//!
//! ## Why not octocrab / a `reqwest` dance
//!
//! - `gh` ships in the dev environment already and is the same client
//!   the user authenticates with for every PR / issue / release op.
//!   Re-authenticating from a Rust binary would need a PAT in the env,
//!   which is friction for zero gain.
//! - octocrab adds ~80 dependencies to the xtask crate's compile
//!   graph. The xtask binary stays sub-second to rebuild today; we'd
//!   like to keep it that way.
//! - The data shape is small (one workflow run = one JSON blob with
//!   <100 step records). `serde_json` over the `gh` stdout is enough.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::process::Command;
use std::str;

use clap::Args;
use serde::Deserialize;

#[derive(Deserialize)]
struct RunRow {
    #[serde(rename = "databaseId")]
    id: u64,
}

const DEFAULT_OWNER: &str = "P4suta";
const DEFAULT_REPO: &str = "aozora";
const DEFAULT_WORKFLOW: &str = "ci";
const DEFAULT_BRANCH: &str = "main";

#[derive(Args)]
pub(crate) struct ProfileArgs {
    /// Workflow file basename (without `.yml`). Default: `ci`.
    #[arg(long, default_value = DEFAULT_WORKFLOW)]
    workflow: String,

    /// Branch to inspect. Default: `main`.
    #[arg(long, default_value = DEFAULT_BRANCH)]
    branch: String,

    /// `OWNER/REPO`. Default: `P4suta/aozora`.
    #[arg(long, default_value_t = format!("{DEFAULT_OWNER}/{DEFAULT_REPO}"))]
    repo: String,

    /// Specific run id. Overrides `--workflow` / `--branch` lookup.
    #[arg(long)]
    run_id: Option<u64>,

    /// How many recent runs to summarise. Reports the median per job.
    /// Ignored when `--run-id` is given.
    #[arg(long, default_value_t = 1)]
    limit: u32,

    /// Show top-N slowest steps across all jobs in the run.
    #[arg(long, default_value_t = 15)]
    top_steps: usize,
}

pub(crate) fn run(args: &ProfileArgs) -> Result<(), String> {
    let runs = match args.run_id {
        Some(id) => vec![id],
        None => list_runs(&args.repo, &args.workflow, &args.branch, args.limit)?,
    };

    if runs.is_empty() {
        return Err(format!(
            "no completed runs found for workflow={} branch={}",
            args.workflow, args.branch
        ));
    }

    if runs.len() == 1 {
        single(&args.repo, runs[0], args.top_steps)
    } else {
        multi(&args.repo, &runs, args.top_steps)
    }
}

// --- single-run report -------------------------------------------------

fn single(repo: &str, run_id: u64, top_steps: usize) -> Result<(), String> {
    let jobs = fetch_jobs(repo, run_id)?;
    println!();
    println!("ci profile  repo={repo}  run_id={run_id}");
    println!(
        "            jobs={}  total_wall={:>6}s",
        jobs.len(),
        jobs.iter().filter_map(Job::duration_secs).sum::<i64>()
    );
    println!();
    print_per_job(&jobs);
    println!();
    print_top_steps(&jobs, top_steps);
    Ok(())
}

fn print_per_job(jobs: &[Job]) {
    println!("{:<36} {:<12} {:>10}", "JOB", "CONCLUSION", "WALL (s)");
    println!("{}", "-".repeat(60));
    let mut sorted: Vec<_> = jobs.iter().collect();
    sorted.sort_by_key(|j| Reverse(j.duration_secs().unwrap_or(0)));
    for j in sorted {
        let dur = j
            .duration_secs()
            .map_or_else(|| "  pending".to_owned(), |d| format!("{d:>10}"));
        let conc = j.conclusion.as_deref().unwrap_or("?");
        println!("{:<36} {:<12} {dur}", trim(&j.name, 36), trim(conc, 12));
    }
}

fn print_top_steps(jobs: &[Job], top_n: usize) {
    let mut all: Vec<(&str, &str, i64, &str)> = jobs
        .iter()
        .flat_map(|j| {
            j.steps.iter().filter_map(move |s| {
                let d = s.duration_secs()?;
                (d > 1).then(|| {
                    (
                        j.name.as_str(),
                        s.name.as_str(),
                        d,
                        s.conclusion.as_deref().unwrap_or("?"),
                    )
                })
            })
        })
        .collect();
    all.sort_by_key(|t| Reverse(t.2));

    println!(
        "{:<28} {:<48} {:<10} {:>10}",
        "JOB", "STEP", "CONCL", "WALL (s)"
    );
    println!("{}", "-".repeat(100));
    for (job, step, dur, conc) in all.into_iter().take(top_n) {
        println!(
            "{:<28} {:<48} {:<10} {dur:>10}",
            trim(job, 28),
            trim(step, 48),
            trim(conc, 10)
        );
    }
}

// --- multi-run report --------------------------------------------------

fn multi(repo: &str, runs: &[u64], top_steps: usize) -> Result<(), String> {
    println!();
    println!("ci profile  repo={repo}  runs={runs:?}  (median across runs per job)");
    println!();

    let mut acc: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    let mut step_acc: BTreeMap<(String, String), Vec<i64>> = BTreeMap::new();
    for &id in runs {
        let jobs = fetch_jobs(repo, id)?;
        for j in &jobs {
            if let Some(d) = j.duration_secs() {
                acc.entry(j.name.clone()).or_default().push(d);
            }
            for s in &j.steps {
                if let Some(d) = s.duration_secs()
                    && d > 1
                {
                    step_acc
                        .entry((j.name.clone(), s.name.clone()))
                        .or_default()
                        .push(d);
                }
            }
        }
    }

    let mut rows: Vec<(&str, i64, usize)> = acc
        .iter()
        .map(|(n, ds)| (n.as_str(), median(ds), ds.len()))
        .collect();
    rows.sort_by_key(|t| Reverse(t.1));

    println!("{:<36} {:>10} {:>10}", "JOB", "MEDIAN(s)", "SAMPLES");
    println!("{}", "-".repeat(60));
    for (n, m, c) in rows {
        println!("{:<36} {m:>10} {c:>10}", trim(n, 36));
    }

    let mut step_rows: Vec<(&str, &str, i64, usize)> = step_acc
        .iter()
        .map(|((j, s), ds)| (j.as_str(), s.as_str(), median(ds), ds.len()))
        .collect();
    step_rows.sort_by_key(|t| Reverse(t.2));

    println!();
    println!(
        "{:<28} {:<48} {:>10} {:>8}",
        "JOB", "STEP", "MEDIAN(s)", "RUNS"
    );
    println!("{}", "-".repeat(100));
    for (j, s, m, c) in step_rows.into_iter().take(top_steps) {
        println!("{:<28} {:<48} {m:>10} {c:>8}", trim(j, 28), trim(s, 48));
    }
    Ok(())
}

fn median(xs: &[i64]) -> i64 {
    if xs.is_empty() {
        return 0;
    }
    let mut s = xs.to_vec();
    s.sort_unstable();
    let mid = s.len() / 2;
    if s.len() % 2 == 1 {
        s[mid]
    } else {
        // `i64::midpoint` (stable since 1.85) computes the rounded
        // average without overflow even for adversarial inputs.
        // No `as` casts, no manual saturating dance, no panic edge
        // cases — the right way to write the operation.
        i64::midpoint(s[mid - 1], s[mid])
    }
}

// --- gh-cli wrappers ---------------------------------------------------

fn list_runs(repo: &str, workflow: &str, branch: &str, limit: u32) -> Result<Vec<u64>, String> {
    // `gh run list --workflow=<wf> --branch=<br> --limit=N
    //  --status completed --json databaseId`
    let out = Command::new("gh")
        .args([
            "run",
            "list",
            "--workflow",
            &format!("{workflow}.yml"),
            "--branch",
            branch,
            "--repo",
            repo,
            "--status",
            "completed",
            "--limit",
            &limit.to_string(),
            "--json",
            "databaseId",
        ])
        .output()
        .map_err(|e| format!("failed to spawn `gh`: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "gh run list failed: {}\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let v: Vec<RunRow> =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("parse gh output: {e}"))?;
    Ok(v.into_iter().map(|r| r.id).collect())
}

#[derive(Deserialize)]
struct JobsResponse {
    jobs: Vec<Job>,
}

#[derive(Deserialize)]
struct Job {
    name: String,
    conclusion: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
    #[serde(default)]
    steps: Vec<Step>,
}

#[derive(Deserialize)]
struct Step {
    name: String,
    conclusion: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
}

impl Job {
    fn duration_secs(&self) -> Option<i64> {
        secs(self.started_at.as_deref(), self.completed_at.as_deref())
    }
}

impl Step {
    fn duration_secs(&self) -> Option<i64> {
        secs(self.started_at.as_deref(), self.completed_at.as_deref())
    }
}

fn fetch_jobs(repo: &str, run_id: u64) -> Result<Vec<Job>, String> {
    let out = Command::new("gh")
        .args([
            "api",
            "--paginate",
            &format!("repos/{repo}/actions/runs/{run_id}/jobs?per_page=100"),
        ])
        .output()
        .map_err(|e| format!("failed to spawn `gh`: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "gh api failed: {}\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    // `gh api --paginate` concatenates per-page JSON objects.
    // Parse each JsonObject sequentially so we accumulate all pages.
    let stream = serde_json::Deserializer::from_slice(&out.stdout).into_iter::<JobsResponse>();
    let mut all = Vec::new();
    for item in stream {
        let resp = item.map_err(|e| format!("parse jobs response: {e}"))?;
        all.extend(resp.jobs);
    }
    Ok(all)
}

// --- timestamp helper --------------------------------------------------

/// `started_at` and `completed_at` are RFC 3339 with `Z` suffix.
/// We avoid pulling `chrono` for this — the only operation needed is
/// "subtract one timestamp from another" and the format is fixed.
fn secs(start: Option<&str>, end: Option<&str>) -> Option<i64> {
    let s = parse_rfc3339_z(start?)?;
    let e = parse_rfc3339_z(end?)?;
    Some(e - s)
}

/// Parse `YYYY-MM-DDTHH:MM:SSZ` into seconds-since-epoch (UTC).
///
/// Skips the `chrono` dependency: GitHub's Actions API always returns
/// the canonical Z-suffixed form, and we only need a difference, not
/// timezone-aware arithmetic. ~30 LOC of date math beats a 200 KB
/// transitive dep.
fn parse_rfc3339_z(s: &str) -> Option<i64> {
    // Expected shape: 2026-04-30T11:36:50Z
    if s.len() != 20 || s.as_bytes()[19] != b'Z' {
        return None;
    }
    let bs = s.as_bytes();
    let n = |a: usize, b: usize| -> Option<i64> { str::from_utf8(&bs[a..b]).ok()?.parse().ok() };
    let year = n(0, 4)?;
    let month = n(5, 7)?;
    let day = n(8, 10)?;
    let hour = n(11, 13)?;
    let min = n(14, 16)?;
    let sec = n(17, 19)?;
    Some(days_from_civil(year, month, day)? * 86_400 + hour * 3600 + min * 60 + sec)
}

/// Howard Hinnant's "days from civil" algorithm — proleptic Gregorian
/// days since 1970-01-01. Closed-form, no leap-year branching beyond
/// the algorithm itself, no tables. Public domain.
/// <https://howardhinnant.github.io/date_algorithms.html#days_from_civil>
///
/// Returns `None` if the inputs cannot represent a real date in the
/// algorithm's domain (year-of-era falls outside `0..=399`, month-of-year
/// outside `0..=11`, or day outside `1..=31`). The caller surfaces that
/// as a parse failure on the timestamp.
fn days_from_civil(y: i64, m: i64, d: i64) -> Option<i64> {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    // yoe ∈ 0..=399 by construction (div_euclid pins the residue).
    let yoe = u32::try_from(y - era * 400).ok()?;
    // mp = "month with March=0, ..., February=11" (Hinnant's shifted month).
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let mp_u32 = u32::try_from(mp).ok()?;
    let d_u32 = u32::try_from(d).ok()?;
    let doy = (153 * mp_u32 + 2) / 5 + d_u32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + i64::from(doe) - 719_468)
}

// --- helpers -----------------------------------------------------------

fn trim(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_owned()
    } else {
        let mut t: String = s.chars().take(max - 1).collect();
        t.push('…');
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_z_round_trip() {
        // 2026-04-30T00:00:00Z = days_since_epoch(2026,4,30) * 86400
        // days_from_civil(2026,4,30) computed below; sanity by diff.
        let a = parse_rfc3339_z("2026-04-30T11:32:25Z").unwrap();
        let b = parse_rfc3339_z("2026-04-30T11:36:50Z").unwrap();
        assert_eq!(b - a, 4 * 60 + 25);
    }

    #[test]
    fn rfc3339_z_handles_year_boundary() {
        let a = parse_rfc3339_z("2025-12-31T23:59:50Z").unwrap();
        let b = parse_rfc3339_z("2026-01-01T00:00:10Z").unwrap();
        assert_eq!(b - a, 20);
    }

    #[test]
    fn rfc3339_z_rejects_malformed() {
        assert_eq!(parse_rfc3339_z("2026-04-30T11:32:25"), None); // no Z
        assert_eq!(parse_rfc3339_z("not-a-date"), None);
        assert_eq!(parse_rfc3339_z(""), None);
    }

    #[test]
    fn median_odd_even() {
        assert_eq!(median(&[1, 2, 3]), 2);
        // i64 division: (2 + 3) / 2 == 2 (truncated). One-second
        // precision is enough for CI wall-time reporting.
        assert_eq!(median(&[1, 2, 3, 4]), 2);
    }

    #[test]
    fn median_empty_is_zero() {
        assert_eq!(median(&[]), 0);
    }

    #[test]
    fn days_from_civil_known_dates() {
        // 1970-01-01 is the epoch.
        assert_eq!(days_from_civil(1970, 1, 1), Some(0));
        // 2000-01-01 is 30*365 + 7 leaps = 10_950 + 7 = 10_957 days.
        // (1972, 76, 80, 84, 88, 92, 96 — seven leap years 1970→2000.)
        assert_eq!(days_from_civil(2000, 1, 1), Some(10_957));
    }

    #[test]
    fn trim_keeps_short_string() {
        assert_eq!(trim("hello", 10), "hello");
    }

    #[test]
    fn trim_truncates_long_string() {
        assert_eq!(trim("hello world", 6), "hello…");
    }
}
