//! Reports GitHub Actions job and step timings through the `gh` client.

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
const JOB_TABLE_WIDTH: usize = 60;
const STEP_TABLE_WIDTH: usize = 100;
const JOB_NAME_WIDTH: usize = 36;
const CONCLUSION_WIDTH: usize = 12;
const STEP_JOB_NAME_WIDTH: usize = 28;
const STEP_NAME_WIDTH: usize = 48;
const STEP_CONCLUSION_WIDTH: usize = 10;

#[derive(Args)]
pub(crate) struct ProfileArgs {
    /// Workflow file basename without `.yml`.
    #[arg(long, default_value = DEFAULT_WORKFLOW)]
    workflow: String,

    /// Branch to inspect.
    #[arg(long, default_value = DEFAULT_BRANCH)]
    branch: String,

    /// `OWNER/REPO`.
    #[arg(long, default_value_t = format!("{DEFAULT_OWNER}/{DEFAULT_REPO}"))]
    repo: String,

    /// Specific run identifier.
    /// Overrides `--workflow` / `--branch` lookup.
    #[arg(long)]
    run_id: Option<u64>,

    /// How many recent runs to summarise.
    /// Reports the median per job.
    /// Uses a single run when `--run-id` is present.
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

#[expect(
    clippy::print_stdout,
    reason = "render the requested workflow timing report"
)]
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

#[expect(clippy::print_stdout, reason = "render the requested job timing table")]
fn print_per_job(jobs: &[Job]) {
    println!("{:<36} {:<12} {:>10}", "JOB", "CONCLUSION", "WALL (s)");
    println!("{}", "-".repeat(JOB_TABLE_WIDTH));
    let mut sorted: Vec<_> = jobs.iter().collect();
    sorted.sort_by_key(|j| Reverse(j.duration_secs().unwrap_or(0)));
    for j in sorted {
        let dur = j
            .duration_secs()
            .map_or_else(|| "  pending".to_owned(), |d| format!("{d:>10}"));
        let conc = j.conclusion.as_deref().unwrap_or("?");
        println!(
            "{:<36} {:<12} {dur}",
            trim(&j.name, JOB_NAME_WIDTH),
            trim(conc, CONCLUSION_WIDTH)
        );
    }
}

#[expect(
    clippy::print_stdout,
    reason = "render the requested step timing table"
)]
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
    println!("{}", "-".repeat(STEP_TABLE_WIDTH));
    for (job, step, dur, conc) in all.into_iter().take(top_n) {
        println!(
            "{:<28} {:<48} {:<10} {dur:>10}",
            trim(job, STEP_JOB_NAME_WIDTH),
            trim(step, STEP_NAME_WIDTH),
            trim(conc, STEP_CONCLUSION_WIDTH)
        );
    }
}

#[expect(
    clippy::print_stdout,
    reason = "render median workflow timings across runs"
)]
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
    println!("{}", "-".repeat(JOB_TABLE_WIDTH));
    for (n, m, c) in rows {
        println!("{:<36} {m:>10} {c:>10}", trim(n, JOB_NAME_WIDTH));
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
    println!("{}", "-".repeat(STEP_TABLE_WIDTH));
    for (j, s, m, c) in step_rows.into_iter().take(top_steps) {
        println!(
            "{:<28} {:<48} {m:>10} {c:>8}",
            trim(j, STEP_JOB_NAME_WIDTH),
            trim(s, STEP_NAME_WIDTH)
        );
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
        i64::midpoint(s[mid - 1], s[mid])
    }
}

fn list_runs(repo: &str, workflow: &str, branch: &str, limit: u32) -> Result<Vec<u64>, String> {
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
    let stream = serde_json::Deserializer::from_slice(&out.stdout).into_iter::<JobsResponse>();
    let mut all = Vec::new();
    for item in stream {
        let resp = item.map_err(|e| format!("parse jobs response: {e}"))?;
        all.extend(resp.jobs);
    }
    Ok(all)
}

fn secs(start: Option<&str>, end: Option<&str>) -> Option<i64> {
    let s = parse_rfc3339_z(start?)?;
    let e = parse_rfc3339_z(end?)?;
    Some(e - s)
}

fn parse_rfc3339_z(s: &str) -> Option<i64> {
    const TIMESTAMP_LENGTH: usize = 20;
    const UTC_SUFFIX_OFFSET: usize = 19;
    const YEAR_END: usize = 4;
    const MONTH_START: usize = 5;
    const MONTH_END: usize = 7;
    const DAY_START: usize = 8;
    const DAY_END: usize = 10;
    const HOUR_START: usize = 11;
    const HOUR_END: usize = 13;
    const MINUTE_START: usize = 14;
    const MINUTE_END: usize = 16;
    const SECOND_START: usize = 17;
    const SECONDS_PER_DAY: i64 = 86_400;
    const SECONDS_PER_HOUR: i64 = 3600;
    const SECONDS_PER_MINUTE: i64 = 60;
    if s.len() != TIMESTAMP_LENGTH || s.as_bytes()[UTC_SUFFIX_OFFSET] != b'Z' {
        return None;
    }
    let bs = s.as_bytes();
    let n = |a: usize, b: usize| -> Option<i64> { str::from_utf8(&bs[a..b]).ok()?.parse().ok() };
    let year = n(0, YEAR_END)?;
    let month = n(MONTH_START, MONTH_END)?;
    let day = n(DAY_START, DAY_END)?;
    let hour = n(HOUR_START, HOUR_END)?;
    let min = n(MINUTE_START, MINUTE_END)?;
    let sec = n(SECOND_START, UTC_SUFFIX_OFFSET)?;
    Some(
        days_from_civil(year, month, day)? * SECONDS_PER_DAY
            + hour * SECONDS_PER_HOUR
            + min * SECONDS_PER_MINUTE
            + sec,
    )
}

/// Uses [Howard Hinnant's public-domain calendar algorithm](https://howardhinnant.github.io/date_algorithms.html#days_from_civil).
fn days_from_civil(y: i64, m: i64, d: i64) -> Option<i64> {
    const YEARS_PER_ERA: i64 = 400;
    const MARCH_OFFSET: i64 = 3;
    const JANUARY_OFFSET: i64 = 9;
    const DAYS_PER_FIVE_MONTHS: u32 = 153;
    const MONTHS_PER_DAY_CYCLE: u32 = 5;
    const COMMON_YEAR_DAYS: u32 = 365;
    const LEAP_YEAR_INTERVAL: u32 = 4;
    const YEARS_PER_CENTURY: u32 = 100;
    const DAYS_PER_ERA: i64 = 146_097;
    const UNIX_EPOCH_CIVIL_DAYS: i64 = 719_468;
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(YEARS_PER_ERA);
    let yoe = u32::try_from(y - era * YEARS_PER_ERA).ok()?;
    let mp = if m > 2 {
        m - MARCH_OFFSET
    } else {
        m + JANUARY_OFFSET
    };
    let mp_u32 = u32::try_from(mp).ok()?;
    let d_u32 = u32::try_from(d).ok()?;
    let doy = (DAYS_PER_FIVE_MONTHS * mp_u32 + 2) / MONTHS_PER_DAY_CYCLE + d_u32 - 1;
    let doe = yoe * COMMON_YEAR_DAYS + yoe / LEAP_YEAR_INTERVAL - yoe / YEARS_PER_CENTURY + doy;
    Some(era * DAYS_PER_ERA + i64::from(doe) - UNIX_EPOCH_CIVIL_DAYS)
}

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
        assert_eq!(parse_rfc3339_z("2026-04-30T11:32:25"), None);
        assert_eq!(parse_rfc3339_z("not-a-date"), None);
        assert_eq!(parse_rfc3339_z(""), None);
    }

    #[test]
    fn median_odd_even() {
        assert_eq!(median(&[1, 2, 3]), 2);
        assert_eq!(median(&[1, 2, 3, 4]), 2);
    }

    #[test]
    fn median_empty_is_zero() {
        assert_eq!(median(&[]), 0);
    }

    #[test]
    fn days_from_civil_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), Some(0));
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

    #[test]
    fn trim_handles_multibyte_chars() {
        assert_eq!(trim("あいうえお", 3), "あい…");
    }

    #[test]
    fn secs_subtracts_two_timestamps() {
        let d = secs(Some("2026-04-30T11:32:25Z"), Some("2026-04-30T11:36:50Z"));
        assert_eq!(d, Some(4 * 60 + 25), "duration in seconds");
    }

    #[test]
    fn secs_is_none_when_either_bound_missing() {
        assert_eq!(secs(None, Some("2026-04-30T11:36:50Z")), None, "no start");
        assert_eq!(secs(Some("2026-04-30T11:32:25Z"), None), None, "no end");
    }

    #[test]
    fn secs_is_none_on_malformed_timestamp() {
        assert_eq!(secs(Some("bad"), Some("2026-04-30T11:36:50Z")), None);
    }

    fn job(name: &str, start: Option<&str>, end: Option<&str>, steps: Vec<Step>) -> Job {
        Job {
            name: name.to_owned(),
            conclusion: Some("success".to_owned()),
            started_at: start.map(ToOwned::to_owned),
            completed_at: end.map(ToOwned::to_owned),
            steps,
        }
    }

    fn step(name: &str, start: Option<&str>, end: Option<&str>) -> Step {
        Step {
            name: name.to_owned(),
            conclusion: Some("success".to_owned()),
            started_at: start.map(ToOwned::to_owned),
            completed_at: end.map(ToOwned::to_owned),
        }
    }

    #[test]
    fn job_duration_secs_computes_from_timestamps() {
        let j = job(
            "build",
            Some("2026-04-30T11:00:00Z"),
            Some("2026-04-30T11:02:00Z"),
            Vec::new(),
        );
        assert_eq!(j.duration_secs(), Some(120), "two-minute job");
    }

    #[test]
    fn job_duration_secs_pending_when_incomplete() {
        let j = job("running", Some("2026-04-30T11:00:00Z"), None, Vec::new());
        assert_eq!(j.duration_secs(), None, "no completed_at → pending");
    }

    #[test]
    fn step_duration_secs_computes_from_timestamps() {
        let s = step(
            "checkout",
            Some("2026-04-30T11:00:00Z"),
            Some("2026-04-30T11:00:30Z"),
        );
        assert_eq!(s.duration_secs(), Some(30), "thirty-second step");
    }

    #[test]
    fn print_per_job_runs_for_mixed_completion() {
        let jobs = vec![
            job(
                "lint",
                Some("2026-04-30T11:00:00Z"),
                Some("2026-04-30T11:01:00Z"),
                Vec::new(),
            ),
            job("pending", Some("2026-04-30T11:00:00Z"), None, Vec::new()),
        ];
        print_per_job(&jobs);
    }

    #[test]
    fn print_top_steps_runs_and_filters_trivial() {
        let jobs = vec![job(
            "build",
            Some("2026-04-30T11:00:00Z"),
            Some("2026-04-30T11:00:10Z"),
            vec![
                step(
                    "quick",
                    Some("2026-04-30T11:00:00Z"),
                    Some("2026-04-30T11:00:01Z"),
                ),
                step(
                    "slow",
                    Some("2026-04-30T11:00:01Z"),
                    Some("2026-04-30T11:00:06Z"),
                ),
            ],
        )];
        print_top_steps(&jobs, 10);
    }
}
