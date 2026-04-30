//! Host-side dev tooling for the aozora workspace.
//!
//! Today: `xtask samply <doc|corpus> …` wrappers around the
//! [`samply`](https://github.com/mstange/samply) sampling profiler.
//!
//! ## Why not a shell script
//!
//! An earlier attempt sat in `scripts/samply-doc.sh` /
//! `scripts/samply-corpus.sh`. It was rewritten in Rust because:
//! - the rest of the project is Rust 2024 — keeping tooling in the
//!   same language means one toolchain, one set of types, one set of
//!   error messages
//! - shell scripts add a parallel shell-quoting / `set -euo pipefail`
//!   surface that is fundamentally harder to reason about
//! - portability: bash idioms break on Windows / non-bash shells; an
//!   `xtask` binary works wherever `cargo run` does
//!
//! ## Why on the host (not Docker)
//!
//! `samply` opens `perf_event_open(2)` directly against the kernel.
//! Docker's default seccomp profile blocks it; even with
//! `--privileged --pid=host` the kernel's `/proc/sys/kernel/perf_event_paranoid`
//! is read inside the container's PID namespace, which doesn't
//! match what the host's perf-events subsystem will allow.
//! Bottom line: profiling needs to be on the host, period.
//!
//! ## Why a separate crate (and not part of `aozora-bench`)
//!
//! `aozora-bench` is a library + examples crate consumed by `cargo
//! bench` and `cargo run --example`. Adding a binary target to it
//! would tie the bench compile to a binary that nobody benchmarking
//! actually wants. The `xtask` pattern keeps developer tooling in a
//! dedicated crate that is not built by `just build`'s default path.
//! The crate is `publish = false` since it's not a library
//! consumers depend on.

#![allow(
    clippy::disallowed_methods,
    reason = "xtask binary uses std::process::exit / std::env::set_var to wire up the spawned `cargo` and `samply` invocations; both are appropriate here, in the dev-tooling crate, but disallowed elsewhere"
)]

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command, ExitStatus};

use clap::{Args, Parser, Subcommand};

mod ci;
mod corpus;
mod deps;
mod trace;

pub(crate) use ci::CiArgs;
pub(crate) use corpus::CorpusArgs;
pub(crate) use deps::DepsArgs;
pub(crate) use trace::TraceArgs;

const PERF_PARANOID_PATH: &str = "/proc/sys/kernel/perf_event_paranoid";
const PERF_PARANOID_MAX: i32 = 1;
const SAMPLY_RATE_HZ: u32 = 4000;
const DEFAULT_CORPUS_REPEAT: usize = 5;
const DEFAULT_RENDER_REPEAT: usize = 5;

#[derive(Parser)]
#[command(name = "xtask", about = "aozora developer tooling", version)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Sample-profile a target via `samply`.
    Samply(SamplyArgs),
    /// Analyse a saved samply `.json.gz` trace via `aozora-trace`.
    Trace(TraceArgs),
    /// Local-only dependency-follow-up tooling — install / inspect /
    /// remove the systemd user timer that runs `just deps-check`
    /// weekly. Replaces the dependabot / GitHub-Actions pattern with
    /// a host-side pure-Rust mechanism.
    Deps(DepsArgs),
    /// Build / inspect aozora-corpus binary archives. Replaces the
    /// directory-of-17-k-small-files load shape with a single packed
    /// file that can be raw SJIS, pre-decoded UTF-8, and/or zstd-
    /// compressed. The pack step is incremental — entries whose source
    /// `mtime` and `blake3` hash match the previous archive are copied
    /// verbatim.
    Corpus(CorpusArgs),
    /// CI / GitHub Actions instrumentation: profile a finished run,
    /// run every CI job locally before pushing, or replay a workflow
    /// job through `nektos/act`.
    Ci(CiArgs),
}

#[derive(Args)]
struct SamplyArgs {
    #[command(subcommand)]
    target: SamplyTarget,
}

#[derive(Subcommand)]
enum SamplyTarget {
    /// Profile a single corpus document via the `pathological_probe` example.
    ///
    /// The probe runs `lex_into_arena` 100 times on the doc, so a 232 KB
    /// outlier doc gives samply ~170 ms of parser-bound wall time at 4 kHz
    /// = ~700 samples. Larger docs (e.g. 3 MB doc 50685) give richer
    /// traces (~10 k samples).
    Doc {
        /// Corpus-relative path under `AOZORA_CORPUS_ROOT`.
        ///
        /// e.g. `001529/files/50685_ruby_67979/50685_ruby_67979.txt`.
        relative_path: String,

        /// Output basename (the `.json.gz` is appended). Defaults to
        /// `aozora-doc-<file-stem>` so multiple runs on different docs
        /// don't clobber each other.
        #[arg(long)]
        out_name: Option<String>,
    },
    /// Profile the parser hot path across the full corpus via the
    /// `throughput_by_class` example.
    ///
    /// `repeat` controls how many times the parse pass is replayed
    /// after the corpus is loaded — higher values give samply more
    /// parser-bound wall time to attach to (the corpus load happens
    /// once and contributes mostly Shift-JIS decode + filesystem
    /// syscalls, which would otherwise dominate the trace).
    Corpus {
        /// Number of parse passes after the one-time corpus load.
        #[arg(default_value_t = DEFAULT_CORPUS_REPEAT)]
        repeat: usize,
    },
    /// Profile the **HTML render** hot path across the full corpus via
    /// the `render_hot_path` example.
    ///
    /// `repeat` controls the per-doc render loop count. Default 5 so
    /// render-bound stack frames dominate the trace; the per-doc parse
    /// (untimed in the probe report but still on the wall) drops to a
    /// minority of samples at this multiplier.
    Render {
        /// Number of `render_to_string` calls per document.
        #[arg(default_value_t = DEFAULT_RENDER_REPEAT)]
        repeat: usize,
    },
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
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
    };
    if let Err(err) = result {
        eprintln!("xtask: {err}");
        process::exit(1);
    }
}

/// Sample-profile a single corpus document via `pathological_probe`.
fn samply_doc(relative_path: &str, out_name: Option<&str>) -> Result<(), String> {
    let corpus_root = require_env("AOZORA_CORPUS_ROOT")?;
    let doc_full = Path::new(&corpus_root).join(relative_path);
    if !doc_full.is_file() {
        return Err(format!("doc not found at {}", doc_full.display()));
    }
    require_perf_paranoid()?;

    let basename = out_name.map_or_else(|| derive_basename(relative_path), str::to_owned);
    let out = PathBuf::from("/tmp").join(format!("aozora-doc-{basename}.json.gz"));

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

/// Sample-profile the corpus parser hot path via `throughput_by_class`.
fn samply_corpus(repeat: usize) -> Result<(), String> {
    require_env("AOZORA_CORPUS_ROOT")?;
    require_perf_paranoid()?;

    let timestamp = current_yyyymmdd_hhmmss();
    let out = PathBuf::from("/tmp").join(format!("aozora-corpus-{timestamp}.json.gz"));

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

/// Sample-profile the HTML render hot path via `render_hot_path`.
/// `repeat` controls per-doc render-loop iterations so render frames
/// dominate the trace over the per-doc parse warmup.
fn samply_render(repeat: usize) -> Result<(), String> {
    require_env("AOZORA_CORPUS_ROOT")?;
    require_perf_paranoid()?;

    let timestamp = current_yyyymmdd_hhmmss();
    let out = PathBuf::from("/tmp").join(format!("aozora-render-{timestamp}.json.gz"));

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

/// Refuse to launch samply when `perf_event_paranoid` is too high.
///
/// Samply uses `perf_event_open(2)` to sample the CPU. The Linux
/// kernel hides that syscall behind `kernel.perf_event_paranoid`,
/// which on most distros defaults to `2` ("block all unprivileged
/// perf access") — samply will spawn but record zero samples.
///
/// We catch this *before* spawning samply because samply itself
/// fails late and silently (a half-empty trace looks like "your
/// program ran too fast"). The error message gives the user a
/// one-shot fix, a permanent fix, and a "why this is needed"
/// explanation in 12 lines or less.
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

/// Format the user-facing message shown when `perf_event_paranoid`
/// is too high. Extracted from [`require_perf_paranoid`] for testing
/// + so the layout is reviewable in isolation.
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

/// Rebuild a bench example with debug info preserved so samply can
/// symbolicate the resulting binary. `--profile=bench` inherits from
/// `release` but overrides `strip = "none"` and `debug = 1`.
///
/// We deliberately do NOT use `cargo run --release` here — that
/// invocation strips debug info and clobbers any prior bench-profile
/// build of the same binary, which is the foot-gun the original
/// shell script existed to avoid.
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

/// Resolve the on-disk path of the bench-profile example binary.
/// Cargo writes profile=bench output to `target/release/examples/`
/// (the `release` directory is shared with `--release`; bench layers
/// on debug info via `[profile.bench] strip = "none"; debug = 1`).
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

/// Walk up from the binary's invocation directory to find the
/// workspace `Cargo.toml`. Cargo sets `CARGO_MANIFEST_DIR` for the
/// xtask crate, so the workspace root is the parent of the
/// `crates/` directory above us.
fn workspace_root() -> Result<PathBuf, String> {
    let manifest_dir = env::var_os("CARGO_MANIFEST_DIR").ok_or_else(|| {
        "CARGO_MANIFEST_DIR not set (xtask must be run via `cargo run -p aozora-xtask`)".to_owned()
    })?;
    let manifest_dir = PathBuf::from(manifest_dir);
    // `crates/aozora-xtask/Cargo.toml` → workspace root is two up.
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
    // Strip the directory + `.txt` suffix → just the file stem.
    Path::new(relative_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("doc")
        .to_owned()
}

/// `YYYYMMDD-HHMMSS` derived from `SystemTime` without pulling in
/// `chrono` for a single invocation. Day-precision is enough to
/// disambiguate per-session profile runs in `/tmp`.
fn current_yyyymmdd_hhmmss() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // Convert to UTC broken-down time using the gregorian-day algorithm.
    // Good enough for filenames; not for actual datetime work.
    let (year, month, day, hour, minute, second) = secs_to_utc(secs);
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}")
}

/// Tiny epoch-seconds → (Y, M, D, h, m, s) for filename timestamps.
/// Doesn't handle leap seconds or pre-1970 inputs (irrelevant here).
#[allow(
    clippy::cast_possible_truncation,
    reason = "epoch sub-day quantities and the day index fit in u32; explicit `as u32` is the simplest expression for this throwaway date-format helper"
)]
fn secs_to_utc(mut secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let second = (secs % 60) as u32;
    secs /= 60;
    let minute = (secs % 60) as u32;
    secs /= 60;
    let hour = (secs % 24) as u32;
    secs /= 24;
    // `secs` is now days since 1970-01-01 (Thursday).
    let mut days = secs as u32;
    let mut year: u32 = 1970;
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
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

fn days_in_year(y: u32) -> u32 {
    if is_leap_year(y) { 366 } else { 365 }
}

fn days_in_month(y: u32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(y) {
                29
            } else {
                28
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
        // 2020-02-29 12:00:00 UTC = 1582977600
        assert_eq!(secs_to_utc(1_582_977_600), (2020, 2, 29, 12, 0, 0));
    }

    #[test]
    fn secs_to_utc_handles_recent_date() {
        // 2026-01-01 00:00:00 UTC = 1767225600
        assert_eq!(secs_to_utc(1_767_225_600), (2026, 1, 1, 0, 0, 0));
    }

    #[test]
    fn paranoid_blocked_message_lists_three_remedies() {
        let msg = format_paranoid_blocked(2);
        // The message MUST tell the user what the problem is and
        // give them at least a one-shot fix + a permanent fix +
        // an explanation of *why* this is needed.
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
