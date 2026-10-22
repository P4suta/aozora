//! Provides development and verification commands for the aozora workspace.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command, ExitStatus};

use clap::{Args, Parser, Subcommand, ValueEnum};

mod artifacts;
mod ci;
mod conformance;
mod coords;
mod corpus;
mod deps;
mod grammar;
mod msrv;
mod perf;
mod publish;
mod ratchet;
mod real_work;
mod release;
mod scan;
mod schema;
mod spec_vectors;
mod trace;
mod types;
mod version;

pub(crate) use artifacts::ArtifactsArgs;
pub(crate) use ci::CiArgs;
pub(crate) use corpus::CorpusArgs;
pub(crate) use deps::DepsArgs;
pub(crate) use real_work::RealWorkArgs;
pub(crate) use trace::TraceArgs;
pub(crate) use version::VersionArgs;

const PERF_PARANOID_PATH: &str = "/proc/sys/kernel/perf_event_paranoid";
const PERF_PARANOID_MAX: i32 = 1;
const SAMPLY_RATE_HZ: u32 = 4000;
const DEFAULT_CORPUS_REPEAT: usize = 5;
const DEFAULT_RENDER_REPEAT: usize = 5;
const SAMPLY_OUTPUT_DIR: &str = "/tmp";

#[derive(Parser)]
#[command(name = "xtask", about = "aozora developer tooling", version)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build and verify local distribution artifacts without publishing them.
    Artifacts(ArtifactsArgs),
    /// Profile a target with `samply`.
    Samply(SamplyArgs),
    /// Analyse a saved samply `.json.gz` trace via `aozora-trace`.
    Trace(TraceArgs),
    /// Manage the local weekly dependency-check timer.
    Deps(DepsArgs),
    /// Build and inspect corpus archives.
    Corpus(CorpusArgs),
    /// Report GitHub Actions run timings.
    Ci(CiArgs),
    /// Generate and check wire schemas.
    Schema(SchemaArgs),
    /// Generate and check binding types.
    Types(TypesArgs),
    /// Run conformance fixtures and check generated grammar artifacts.
    Conformance(ConformanceArgs),
    /// Print the build version for the selected channel.
    Version(VersionArgs),
    /// Synchronize and check vendored specification vectors.
    SpecVectors(SpecVectorsArgs),
    /// Check publishable manifests and changelog coverage offline.
    Publish(PublishArgs),
    /// Check release contracts and rehearse publishers.
    Release(ReleaseArgs),
    /// Run deterministic instruction-count performance contracts.
    Perf(PerfArgs),
    /// Enforce monotonic performance, allocation, artifact, and wire baselines.
    Ratchet(RatchetArgs),
    /// Assemble immutable real-work release-lab inputs and worker bundles.
    RealWork(RealWorkArgs),
    /// Check the supported Rust version policy and toolchain pins.
    Msrv(MsrvArgs),
    /// Check repository source policy.
    Lint(LintArgs),
}

#[derive(Args)]
struct LintArgs {
    #[command(subcommand)]
    op: LintOp,
}

#[derive(Subcommand)]
enum LintOp {
    /// Reject source coordinates in comments.
    Coordinates,
}

#[derive(Args)]
struct MsrvArgs {
    #[command(subcommand)]
    op: MsrvOp,
}

#[derive(Subcommand)]
enum MsrvOp {
    /// Check toolchain pins and the six-month supported Rust version policy.
    Check,
}

#[derive(Args)]
struct PublishArgs {
    #[command(subcommand)]
    op: PublishOp,
}

#[derive(Args)]
struct PerfArgs {
    #[command(subcommand)]
    op: PerfOp,
}

#[derive(Subcommand)]
enum PerfOp {
    /// Run every committed Callgrind workload and enforce edit speedups.
    Check,
}

#[derive(Args)]
struct RatchetArgs {
    /// Git commit containing the previous baselines.
    #[arg(long, env = "BASELINE_RATCHET_BASE", default_value = "HEAD")]
    base: String,
}

#[derive(Subcommand)]
enum PublishOp {
    /// Reject manifest or changelog coverage drift.
    Check,
}

#[derive(Args)]
struct ReleaseArgs {
    #[command(subcommand)]
    op: ReleaseOp,
}

#[derive(Subcommand)]
enum ReleaseOp {
    /// Decide whether the checked-out commit should publish.
    /// Prints `release=<bool>` for `$GITHUB_OUTPUT`.
    RearmDecision {
        /// Triggering event name.
        #[arg(long, env = "GITHUB_EVENT_NAME")]
        event: String,
        /// Whether `HEAD` bumped the workspace version, as `true` or `false`.
        #[arg(long, env = "VERSION_CHANGED")]
        version_changed: String,
        /// Qualified commit, which must equal `HEAD`.
        #[arg(long, env = "QUALIFIED_SHA")]
        commit: String,
    },
    /// Check release rules and artifact path consistency offline.
    Check,
    /// Verify the rearm preconditions.
    /// Uses network probes unless `--offline` disables them.
    Preflight {
        /// Skip network probes and run local checks.
        #[arg(long)]
        offline: bool,
        /// Commit to rearm, defaulting to `HEAD`.
        #[arg(long)]
        commit: Option<String>,
        /// Acknowledge a first publication that requires manual registry setup.
        #[arg(long)]
        first_publish: bool,
    },
    /// Dispatch publisher rehearsals and reject qualification failures or zero dispatches.
    Rehearse {
        /// Commit to rehearse, defaulting to `HEAD`.
        #[arg(long)]
        commit: Option<String>,
    },
}

#[derive(Args)]
struct SpecVectorsArgs {
    #[command(subcommand)]
    op: SpecVectorsOp,
}

#[derive(Subcommand)]
enum SpecVectorsOp {
    /// Reject drift from the configured specification source.
    Check,
    /// Replace vendored vectors and supporting files from the configured specification source.
    Sync,
}

#[derive(Args)]
struct SchemaArgs {
    #[command(subcommand)]
    op: SchemaOp,
}

#[derive(Subcommand)]
enum SchemaOp {
    /// Regenerate wire schemas in `crates/aozora-conformance/json/`.
    Dump,
    /// Compare committed schemas with generated output and reject drift.
    Check,
}

#[derive(Args)]
struct ConformanceArgs {
    #[command(subcommand)]
    op: ConformanceOp,
}

#[derive(Args)]
struct RunArgs {
    /// Which implementation to measure conformance for.
    #[arg(long, value_enum, default_value_t = Implementation::Rust)]
    implementation: Implementation,
    /// Regenerate tree-sitter snapshots and results.
    /// Has no effect on the `rust` implementation.
    #[arg(long)]
    update: bool,
}

#[derive(Args)]
struct VectorsArgs {
    /// Which implementation to run the specification vectors through.
    #[arg(long, value_enum, default_value_t = Implementation::Rust)]
    implementation: Implementation,
    /// Regenerate tree-sitter specification snapshots.
    /// Has no effect on the `rust` implementation.
    #[arg(long)]
    update: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Implementation {
    /// The canonical Rust parser.
    Rust,
    /// The reference tree-sitter grammar.
    #[value(name = "tree-sitter")]
    TreeSitter,
}

#[derive(Subcommand)]
enum ConformanceOp {
    /// Run fixtures with the selected implementation.
    /// Rust rejects `must` failures, and tree-sitter rejects snapshot drift.
    Run(RunArgs),
    /// Run vendored specification vectors with the selected implementation.
    /// Reject any Rust projection mismatch or tree-sitter snapshot drift.
    Vectors(VectorsArgs),
    /// Check or regenerate the committed tree-sitter parser.
    Grammar(GrammarArgs),
}

#[derive(Args)]
struct GrammarArgs {
    /// Regenerate parser artifacts from `grammar.js` with the pinned tree-sitter tool.
    #[arg(long)]
    update: bool,
    /// Reject generated parser drift.
    /// Runs by default without either flag.
    #[arg(long, conflicts_with = "update")]
    check: bool,
}

#[derive(Args)]
struct TypesArgs {
    #[command(subcommand)]
    op: TypesOp,
}

#[derive(Subcommand)]
enum TypesOp {
    /// Regenerate TypeScript wire types from Rust definitions.
    Ts,
    /// Reject TypeScript wire type drift.
    Check,
    /// Regenerate host binding types from committed schemas with `quicktype`.
    Langs,
    /// Reject host binding type drift.
    LangsCheck,
}

#[derive(Args)]
struct SamplyArgs {
    #[command(subcommand)]
    target: SamplyTarget,
}

#[derive(Subcommand)]
enum SamplyTarget {
    /// Profile a corpus document through the `pathological_probe` example.
    Doc {
        /// Corpus-relative path under `AOZORA_CORPUS_ROOT`.
        relative_path: String,

        /// Output basename without the `.json.gz` suffix.
        /// Defaults to `aozora-doc-<file-stem>`.
        #[arg(long)]
        out_name: Option<String>,
    },
    /// Profile corpus parsing through the `throughput_by_class` example.
    Corpus {
        /// Number of parse passes after the one-time corpus load.
        #[arg(default_value_t = DEFAULT_CORPUS_REPEAT)]
        repeat: usize,
    },
    /// Profile corpus rendering through the `render_hot_path` example.
    Render {
        /// Number of `render_to_string` calls per document.
        #[arg(default_value_t = DEFAULT_RENDER_REPEAT)]
        repeat: usize,
    },
}

#[expect(
    clippy::print_stderr,
    clippy::disallowed_methods,
    reason = "the command entry point reports errors and selects the process exit status"
)]
fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Cmd::Artifacts(args) => artifacts::dispatch(&args),
        Cmd::Samply(args) => match args.target {
            SamplyTarget::Doc {
                relative_path,
                out_name,
            } => samply_doc(&relative_path, out_name.as_deref()),
            SamplyTarget::Corpus { repeat } => samply_corpus(repeat),
            SamplyTarget::Render { repeat } => samply_render(repeat),
        },
        Cmd::Trace(args) => trace::dispatch(args),
        Cmd::Deps(args) => deps::dispatch(&args),
        Cmd::Corpus(args) => corpus::dispatch(&args),
        Cmd::Ci(args) => ci::run(&args),
        Cmd::Schema(args) => schema::dispatch(&args),
        Cmd::Types(args) => types::dispatch(&args),
        Cmd::Conformance(args) => conformance::dispatch(&args),
        Cmd::Version(args) => version::dispatch(&args),
        Cmd::SpecVectors(args) => spec_vectors::dispatch(&args),
        Cmd::Publish(args) => publish::dispatch(&args),
        Cmd::Release(args) => release::dispatch(&args),
        Cmd::Perf(args) => perf::dispatch(&args),
        Cmd::Ratchet(args) => ratchet::check(&args),
        Cmd::RealWork(args) => real_work::dispatch(&args),
        Cmd::Msrv(args) => msrv::dispatch(&args),
        Cmd::Lint(args) => match args.op {
            LintOp::Coordinates => coords::check(),
        },
    };
    if let Err(err) = result {
        eprintln!("xtask: {err}");
        process::exit(1);
    }
}

#[expect(
    clippy::print_stderr,
    reason = "report profiling progress and output paths"
)]
fn samply_doc(relative_path: &str, out_name: Option<&str>) -> Result<(), String> {
    let corpus_root = require_env("AOZORA_CORPUS_ROOT")?;
    let doc_full = Path::new(&corpus_root).join(relative_path);
    if !doc_full.is_file() {
        return Err(format!("doc not found at {}", doc_full.display()));
    }
    require_perf_paranoid()?;

    let basename = out_name.map_or_else(|| derive_basename(relative_path), str::to_owned);
    let out = PathBuf::from(SAMPLY_OUTPUT_DIR).join(format!("aozora-doc-{basename}.json.gz"));

    rebuild_with_debug("pathological_probe")?;
    let bin = bench_example_path("pathological_probe")?;

    eprintln!(
        ">>> samply: doc={relative_path}\n           out={}",
        out.display()
    );
    let status = Command::new("samply")
        .arg("record")
        .arg("--save-only")
        .arg("--no-open")
        .arg("-o")
        .arg(&out)
        .arg("-r")
        .arg(SAMPLY_RATE_HZ.to_string())
        .arg("--")
        .arg(bin)
        .env("AOZORA_PROBE_DOC", relative_path)
        .status()
        .map_err(|e| format!("failed to spawn samply: {e}"))?;
    expect_status(status, "samply record")?;

    eprintln!();
    eprintln!(">>> done. inspect with:");
    eprintln!(
        "    samply load {}        # opens local Firefox-Profiler UI",
        out.display()
    );
    Ok(())
}

#[expect(
    clippy::print_stderr,
    reason = "report profiling progress and output paths"
)]
fn samply_corpus(repeat: usize) -> Result<(), String> {
    require_env("AOZORA_CORPUS_ROOT")?;
    require_perf_paranoid()?;

    let timestamp = current_yyyymmdd_hhmmss();
    let out = PathBuf::from(SAMPLY_OUTPUT_DIR).join(format!("aozora-corpus-{timestamp}.json.gz"));

    rebuild_with_debug("throughput_by_class")?;
    let bin = bench_example_path("throughput_by_class")?;

    eprintln!(
        ">>> samply: repeat={repeat}\n           out={}",
        out.display()
    );
    let status = Command::new("samply")
        .arg("record")
        .arg("--save-only")
        .arg("--no-open")
        .arg("-o")
        .arg(&out)
        .arg("-r")
        .arg(SAMPLY_RATE_HZ.to_string())
        .arg("--")
        .arg(bin)
        .env("AOZORA_PROFILE_REPEAT", repeat.to_string())
        .status()
        .map_err(|e| format!("failed to spawn samply: {e}"))?;
    expect_status(status, "samply record")?;

    eprintln!();
    eprintln!(">>> done. inspect with:");
    eprintln!(
        "    samply load {}        # opens local Firefox-Profiler UI",
        out.display()
    );
    Ok(())
}

#[expect(
    clippy::print_stderr,
    reason = "report profiling progress and output paths"
)]
fn samply_render(repeat: usize) -> Result<(), String> {
    require_env("AOZORA_CORPUS_ROOT")?;
    require_perf_paranoid()?;

    let timestamp = current_yyyymmdd_hhmmss();
    let out = PathBuf::from(SAMPLY_OUTPUT_DIR).join(format!("aozora-render-{timestamp}.json.gz"));

    rebuild_with_debug("render_hot_path")?;
    let bin = bench_example_path("render_hot_path")?;

    eprintln!(
        ">>> samply: repeat={repeat}\n           out={}",
        out.display()
    );
    let status = Command::new("samply")
        .arg("record")
        .arg("--save-only")
        .arg("--no-open")
        .arg("-o")
        .arg(&out)
        .arg("-r")
        .arg(SAMPLY_RATE_HZ.to_string())
        .arg("--")
        .arg(bin)
        .env("AOZORA_RENDER_REPEAT", repeat.to_string())
        .status()
        .map_err(|e| format!("failed to spawn samply: {e}"))?;
    expect_status(status, "samply record")?;

    eprintln!();
    eprintln!(">>> done. inspect with:");
    eprintln!(
        "    samply load {}        # opens local Firefox-Profiler UI",
        out.display()
    );
    Ok(())
}

fn require_env(key: &str) -> Result<OsString, String> {
    env::var_os(key).ok_or_else(|| format!("{key} not set"))
}

fn require_perf_paranoid() -> Result<(), String> {
    let raw = match fs::read_to_string(PERF_PARANOID_PATH) {
        Ok(s) => s,
        Err(e) => {
            return Err(format!(
                "\n\
                 ╭─────────────────────────────────────────────────────────────────╮\n\
                 │  ❌  Cannot read {PERF_PARANOID_PATH:46}  │\n\
                 │      ({e:60}) │\n\
                 │                                                                 │\n\
                 │      Samply needs perf_event_open(2). Without this file we      │\n\
                 │      can't tell whether the kernel will allow it. Bailing now.  │\n\
                 ╰─────────────────────────────────────────────────────────────────╯"
            ));
        }
    };
    let level: i32 = raw
        .trim()
        .parse()
        .map_err(|e| format!("failed to parse {PERF_PARANOID_PATH}={raw:?}: {e}"))?;
    if level > PERF_PARANOID_MAX {
        return Err(format_paranoid_blocked(level));
    }
    Ok(())
}

fn format_paranoid_blocked(level: i32) -> String {
    format!(
        "\n\
         ╭──────────────────────────────────────────────────────────────────────╮\n\
         │  🔒  perf_event_paranoid = {level} — samply CANNOT collect samples here. │\n\
         ╰──────────────────────────────────────────────────────────────────────╯\n\
         \n\
         ▸ One-shot fix (resets at next reboot):\n     \
             echo {PERF_PARANOID_MAX} | sudo tee {PERF_PARANOID_PATH}\n\
         \n\
         ▸ Permanent fix (survives reboots):\n     \
             echo 'kernel.perf_event_paranoid = {PERF_PARANOID_MAX}' | sudo tee /etc/sysctl.d/99-perf.conf\n     \
             sudo sysctl --system\n\
         \n\
         ▸ Why this is required:\n     \
             samply uses perf_event_open(2) to sample the CPU at {SAMPLY_RATE_HZ}Hz.\n     \
             The kernel guards that syscall behind perf_event_paranoid; the\n     \
             default of 2 blocks all unprivileged use. samply would otherwise\n     \
             spawn but record zero samples, which looks like 'your program ran\n     \
             too fast' — much harder to diagnose than this message.\n\
         \n\
         ▸ Security note:\n     \
             Setting paranoid=1 lets unprivileged processes profile their own\n     \
             children. Lower than the default 2 but still safer than 0 (which\n     \
             would expose kernel internals). For a single-user dev workstation\n     \
             this is the standard recommendation.\n"
    )
}

#[expect(clippy::print_stderr, reason = "report the profiling build command")]
fn rebuild_with_debug(example: &str) -> Result<(), String> {
    eprintln!(">>> rebuilding {example} with debug info (--profile=bench)");
    let status = Command::new(env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")))
        .arg("build")
        .arg("--profile=bench")
        .arg("--example")
        .arg(example)
        .arg("-p")
        .arg("aozora-bench")
        .status()
        .map_err(|e| format!("failed to spawn cargo: {e}"))?;
    expect_status(status, "cargo build --profile=bench")
}

fn bench_example_path(example: &str) -> Result<PathBuf, String> {
    let workspace = workspace_root()?;
    let path = workspace
        .join("target")
        .join("release")
        .join("examples")
        .join(example);
    if !path.is_file() {
        return Err(format!(
            "expected bench example at {} (build skipped or failed?)",
            path.display()
        ));
    }
    Ok(path)
}

fn workspace_root() -> Result<PathBuf, String> {
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR").ok_or_else(|| {
        "CARGO_MANIFEST_DIR not set (xtask must be run via `cargo run -p aozora-xtask`)".to_owned()
    })?;
    let manifest_dir = PathBuf::from(manifest_dir);
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            format!(
                "cannot derive workspace root from CARGO_MANIFEST_DIR={}",
                manifest_dir.display()
            )
        })
}

fn expect_status(status: ExitStatus, label: &str) -> Result<(), String> {
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label} failed: {status}"))
    }
}

fn derive_basename(relative_path: &str) -> String {
    Path::new(relative_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("doc")
        .to_owned()
}

fn current_yyyymmdd_hhmmss() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (year, month, day, hour, minute, second) = secs_to_utc(secs);
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}")
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "epoch sub-day quantities and the day index fit in u32; explicit `as u32` is the simplest expression for this throwaway date-format helper"
)]
fn secs_to_utc(mut secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    const SECONDS_PER_MINUTE: u64 = 60;
    const MINUTES_PER_HOUR: u64 = 60;
    const HOURS_PER_DAY: u64 = 24;
    const UNIX_EPOCH_YEAR: u32 = 1970;
    let second = (secs % SECONDS_PER_MINUTE) as u32;
    secs /= SECONDS_PER_MINUTE;
    let minute = (secs % MINUTES_PER_HOUR) as u32;
    secs /= MINUTES_PER_HOUR;
    let hour = (secs % HOURS_PER_DAY) as u32;
    secs /= HOURS_PER_DAY;
    let mut days = secs as u32;
    let mut year: u32 = UNIX_EPOCH_YEAR;
    loop {
        let len = days_in_year(year);
        if days < len {
            break;
        }
        days -= len;
        year += 1;
    }
    let mut month: u32 = 1;
    loop {
        let len = days_in_month(year, month);
        if days < len {
            break;
        }
        days -= len;
        month += 1;
    }
    let day = days + 1;
    (year, month, day, hour, minute, second)
}

fn is_leap_year(y: u32) -> bool {
    const LEAP_YEAR_INTERVAL: u32 = 4;
    const YEARS_PER_CENTURY: u32 = 100;
    const GREGORIAN_CYCLE_YEARS: u32 = 400;
    (y.is_multiple_of(LEAP_YEAR_INTERVAL) && !y.is_multiple_of(YEARS_PER_CENTURY))
        || y.is_multiple_of(GREGORIAN_CYCLE_YEARS)
}

fn days_in_year(y: u32) -> u32 {
    const LEAP_YEAR_DAYS: u32 = 366;
    const COMMON_YEAR_DAYS: u32 = 365;
    if is_leap_year(y) {
        LEAP_YEAR_DAYS
    } else {
        COMMON_YEAR_DAYS
    }
}

fn days_in_month(y: u32, m: u32) -> u32 {
    const JANUARY: u32 = 1;
    const FEBRUARY: u32 = 2;
    const MARCH: u32 = 3;
    const APRIL: u32 = 4;
    const MAY: u32 = 5;
    const JUNE: u32 = 6;
    const JULY: u32 = 7;
    const AUGUST: u32 = 8;
    const SEPTEMBER: u32 = 9;
    const OCTOBER: u32 = 10;
    const NOVEMBER: u32 = 11;
    const DECEMBER: u32 = 12;
    const LONG_MONTH_DAYS: u32 = 31;
    const SHORT_MONTH_DAYS: u32 = 30;
    const LEAP_FEBRUARY_DAYS: u32 = 29;
    const COMMON_FEBRUARY_DAYS: u32 = 28;
    match m {
        JANUARY | MARCH | MAY | JULY | AUGUST | OCTOBER | DECEMBER => LONG_MONTH_DAYS,
        APRIL | JUNE | SEPTEMBER | NOVEMBER => SHORT_MONTH_DAYS,
        FEBRUARY => {
            if is_leap_year(y) {
                LEAP_FEBRUARY_DAYS
            } else {
                COMMON_FEBRUARY_DAYS
            }
        }
        _ => unreachable!("month out of range"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_basename_strips_directory_and_extension() {
        assert_eq!(
            derive_basename("001529/files/50685_ruby_67979/50685_ruby_67979.txt"),
            "50685_ruby_67979"
        );
    }

    #[test]
    fn derive_basename_handles_no_extension() {
        assert_eq!(derive_basename("foo/bar"), "bar");
    }

    #[test]
    fn secs_to_utc_unix_epoch_is_1970_01_01() {
        assert_eq!(secs_to_utc(0), (1970, 1, 1, 0, 0, 0));
    }

    #[test]
    fn secs_to_utc_handles_leap_year_crossing() {
        assert_eq!(secs_to_utc(1_582_977_600), (2020, 2, 29, 12, 0, 0));
    }

    #[test]
    fn secs_to_utc_handles_recent_date() {
        assert_eq!(secs_to_utc(1_767_225_600), (2026, 1, 1, 0, 0, 0));
    }

    #[test]
    fn paranoid_blocked_message_lists_three_remedies() {
        let msg = format_paranoid_blocked(2);
        assert!(
            msg.contains("perf_event_paranoid = 2"),
            "missing observed value: {msg}"
        );
        assert!(msg.contains("One-shot fix"));
        assert!(msg.contains("Permanent fix"));
        assert!(msg.contains("/etc/sysctl.d/99-perf.conf"));
        assert!(msg.contains("Why this is required"));
        assert!(msg.contains("perf_event_open(2)"));
        assert!(msg.contains("Security note"));
    }
}
