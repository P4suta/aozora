//! Reports corpus loading and parsing throughput by document size.

mod support;

use support::report_number;

use std::cell::RefCell;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use aozora::decode_auto;
use aozora_bench::{
    SizeBand, SizeBandedCorpus, archive_size_bands_limited, corpus_size_bands_from_decoded,
    parallel_size_bands_limited,
};
use aozora_corpus::{Archive, FilesystemCorpus};
use rayon::prelude::*;

thread_local! {
    static WORKER_ARENA: RefCell<()> = const { RefCell::new(()) };
}

const NS_PER_S: f64 = 1_000_000_000.0;

fn parallel_mode() -> bool {
    matches!(
        env::var("AOZORA_PROFILE_PARALLEL").ok().as_deref(),
        Some("1" | "true" | "yes")
    )
}

fn main() -> ExitCode {
    run().err().unwrap_or(ExitCode::SUCCESS)
}

#[expect(
    clippy::print_stderr,
    clippy::expect_used,
    reason = "benchmark commands report measurements and stop on invalid benchmark inputs"
)]
fn run() -> Result<(), ExitCode> {
    let archive_path = env::var_os("AOZORA_CORPUS_ARCHIVE");
    let root = env::var_os("AOZORA_CORPUS_ROOT");
    if archive_path.is_none() && root.is_none() {
        eprintln!(
            "neither AOZORA_CORPUS_ARCHIVE nor AOZORA_CORPUS_ROOT is set; nothing to profile."
        );
        return Err(ExitCode::from(2));
    }

    let limit: Option<usize> = env::var("AOZORA_PROFILE_LIMIT")
        .ok()
        .and_then(|s| s.trim().parse().ok());
    let repeat: usize = env::var("AOZORA_PROFILE_REPEAT")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1)
        .max(1);

    let parallel = parallel_mode();
    eprintln!(
        "throughput_by_class: starting (limit = {limit:?}, repeat = {repeat}, parallel = {parallel})"
    );

    let load = if let Some(archive_path) = archive_path.as_ref() {
        LoadPhase::run_archive(Path::new(archive_path), limit)
    } else {
        let root = root.expect("AOZORA_CORPUS_ROOT must be set when ARCHIVE is not");
        let Ok(corpus) = FilesystemCorpus::new(PathBuf::from(&root)) else {
            eprintln!("AOZORA_CORPUS_ROOT is not a readable directory; nothing to profile.");
            return Err(ExitCode::from(2));
        };

        if parallel {
            LoadPhase::run_parallel(&corpus, limit)
        } else {
            LoadPhase::run(&corpus, limit)
        }
    };

    print_load_report(&load);
    if load.banded.io_errors != 0 {
        eprintln!(
            "throughput_by_class: refusing a partial-corpus measurement after {} I/O error(s)",
            load.banded.io_errors
        );
        return Err(ExitCode::from(2));
    }

    let parse_start = Instant::now();
    let mut report = measure_all(&load.banded, parallel);
    for _ in 1..repeat {
        report = measure_all(&load.banded, parallel);
    }
    let parse_secs = parse_start.elapsed().as_secs_f64();

    print_report(
        &report,
        &load.banded,
        ReportTiming {
            parse_secs,
            load_secs: load.total_secs,
            repeat,
            parallel,
        },
    );
    Ok(())
}

#[expect(
    clippy::print_stderr,
    reason = "benchmark commands report measurements and failures"
)]
fn print_load_report(load: &LoadPhase) {
    eprintln!(
        "throughput_by_class: bucketed (small={}, medium={}, large={}, path={}, decode_err={}, io_err={})",
        load.banded.small.len(),
        load.banded.medium.len(),
        load.banded.large.len(),
        load.banded.pathological.len(),
        load.banded.decode_errors,
        load.banded.io_errors,
    );
    eprintln!(
        "throughput_by_class: load wall {:.2}s (Shift-JIS decode + I/O + bucketing — \
         excluded from parse measurements)",
        load.total_secs
    );
    if let Some(split) = &load.split {
        eprintln!(
            "  ├─ walkdir : {:>5.2}s ({:>4.1}%, {} paths)",
            split.walkdir_secs,
            100.0 * split.walkdir_secs / load.total_secs,
            split.path_count,
        );
        eprintln!(
            "  ├─ read    : {:>5.2}s ({:>4.1}%, {:.1} MB sjis)",
            split.read_secs,
            100.0 * split.read_secs / load.total_secs,
            report_number(split.sjis_bytes_total) / 1_048_576.0,
        );
        eprintln!(
            "  ├─ decode  : {:>5.2}s ({:>4.1}%, {:.1} MB utf8 → {:.0} MB/s)",
            split.decode_secs,
            100.0 * split.decode_secs / load.total_secs,
            report_number(split.utf8_bytes_total) / 1_048_576.0,
            (report_number(split.sjis_bytes_total) / 1_048_576.0)
                / split.decode_secs.max(f64::EPSILON),
        );
        eprintln!(
            "  └─ bucket  : {:>5.2}s ({:>4.1}%)",
            split.bucket_secs,
            100.0 * split.bucket_secs / load.total_secs,
        );
    } else {
        eprintln!(
            "  └─ parallel mode (rayon): walkdir + per-file read+decode+bucket fanned across workers; \
             sub-phase timers do not attribute"
        );
    }
}

struct LoadPhase {
    banded: SizeBandedCorpus,
    total_secs: f64,
    split: Option<LoadSplit>,
}

struct LoadSplit {
    walkdir_secs: f64,
    read_secs: f64,
    decode_secs: f64,
    bucket_secs: f64,
    path_count: usize,
    sjis_bytes_total: u64,
    utf8_bytes_total: u64,
}

impl LoadPhase {
    fn run(corpus: &FilesystemCorpus, limit: Option<usize>) -> Self {
        let total_start = Instant::now();

        let walk_start = Instant::now();
        let mut paths = Vec::new();
        let mut io_errors = 0;
        for path in corpus.walk_paths().take(limit.unwrap_or(usize::MAX)) {
            match path {
                Ok(path) => paths.push(path),
                Err(_) => io_errors += 1,
            }
        }
        let walkdir_secs = walk_start.elapsed().as_secs_f64();
        let path_count = paths.len();

        let read_start = Instant::now();
        let mut items = Vec::with_capacity(paths.len());
        for path in &paths {
            match corpus.read_path(path) {
                Ok(item) => items.push(item),
                Err(_) => io_errors += 1,
            }
        }
        let read_secs = read_start.elapsed().as_secs_f64();
        let sjis_bytes_total: u64 = items.iter().map(|it| it.bytes.len() as u64).sum();

        let decode_start = Instant::now();
        let mut decoded: Vec<(String, String)> = Vec::with_capacity(items.len());
        let mut decode_errors: usize = 0;
        for item in items {
            let label = item.label;
            let bytes = item.bytes;
            match decode_auto(&bytes) {
                Ok(text) => decoded.push((label, text.into_owned())),
                Err(_) => decode_errors += 1,
            }
        }
        let decode_secs = decode_start.elapsed().as_secs_f64();
        let utf8_bytes_total: u64 = decoded.iter().map(|(_, t)| t.len() as u64).sum();

        let bucket_start = Instant::now();
        let mut banded = corpus_size_bands_from_decoded(decoded);
        banded.decode_errors = decode_errors;
        banded.io_errors = io_errors;
        let bucket_secs = bucket_start.elapsed().as_secs_f64();

        let total_secs = total_start.elapsed().as_secs_f64();

        Self {
            banded,
            total_secs,
            split: Some(LoadSplit {
                walkdir_secs,
                read_secs,
                decode_secs,
                bucket_secs,
                path_count,
                sjis_bytes_total,
                utf8_bytes_total,
            }),
        }
    }

    fn run_parallel(corpus: &FilesystemCorpus, limit: Option<usize>) -> Self {
        let total_start = Instant::now();
        let banded = parallel_size_bands_limited(corpus, limit);
        let total_secs = total_start.elapsed().as_secs_f64();
        Self {
            banded,
            total_secs,
            split: None,
        }
    }

    #[expect(
        clippy::panic,
        clippy::print_stderr,
        reason = "benchmark commands report measurements and stop on invalid benchmark inputs"
    )]
    fn run_archive(path: &Path, limit: Option<usize>) -> Self {
        let total_start = Instant::now();
        let archive =
            Archive::open(path).unwrap_or_else(|err| panic!("open {}: {err}", path.display()));
        let archive_size = fs::metadata(path)
            .unwrap_or_else(|err| panic!("inspect {}: {err}", path.display()))
            .len();
        eprintln!(
            "throughput_by_class: archive {} ({} entries, {}{}, {:.1} MB on disk)",
            path.display(),
            archive.len(),
            if archive.is_utf8() { "UTF8 " } else { "SJIS " },
            if archive.is_zstd() { "ZSTD" } else { "RAW" },
            report_number(archive_size) / 1_048_576.0,
        );
        let banded = archive_size_bands_limited(&archive, limit);
        let total_secs = total_start.elapsed().as_secs_f64();
        Self {
            banded,
            total_secs,
            split: None,
        }
    }
}

#[derive(Debug, Default)]
struct BandReport {
    latencies_ns: Vec<u64>,
    sizes_bytes: Vec<u64>,
}

impl BandReport {
    fn total_bytes(&self) -> u64 {
        self.sizes_bytes.iter().sum()
    }
    fn total_ns(&self) -> u64 {
        self.latencies_ns.iter().sum()
    }
    #[expect(
        clippy::expect_used,
        reason = "benchmark inputs must meet parser and numeric preconditions"
    )]
    fn quantile(sorted: &[u64], p: f64) -> u64 {
        if sorted.is_empty() {
            return 0;
        }
        let idx = ((report_number(sorted.len()) - 1.0) * p)
            .round()
            .to_string()
            .parse::<usize>()
            .expect("quantile index fits usize");
        sorted[idx.min(sorted.len() - 1)]
    }
}

#[derive(Debug, Default)]
struct AllReport {
    bands: [BandReport; 4],
}

fn measure_all(banded: &SizeBandedCorpus, parallel: bool) -> AllReport {
    let mut report = AllReport::default();
    for (slot, band) in SizeBand::ordered().into_iter().enumerate() {
        let docs = banded.band(band);
        report.bands[slot] = measure_band(docs, parallel);
    }
    report
}

#[expect(
    clippy::expect_used,
    reason = "benchmark inputs must meet parser and numeric preconditions"
)]
fn measure_band(docs: &[(String, String)], parallel: bool) -> BandReport {
    let measure = |text: &str| -> (u64, u64) {
        WORKER_ARENA.with(|_cell| {
            let t = Instant::now();
            let _out = aozora::parse(text)
                .expect("source fits parser span limit")
                .snapshot();
            let ns = u64::try_from(t.elapsed().as_nanos()).expect("elapsed nanoseconds fit u64");
            (text.len() as u64, ns)
        })
    };

    let pairs: Vec<(u64, u64)> = if parallel {
        docs.par_iter().map(|(_, text)| measure(text)).collect()
    } else {
        docs.iter().map(|(_, text)| measure(text)).collect()
    };

    let (sizes_bytes, latencies_ns) = pairs.into_iter().unzip();
    BandReport {
        latencies_ns,
        sizes_bytes,
    }
}

#[derive(Clone, Copy)]
struct ReportTiming {
    parse_secs: f64,
    load_secs: f64,
    repeat: usize,
    parallel: bool,
}

#[expect(
    clippy::print_stdout,
    clippy::expect_used,
    reason = "benchmark commands report measurements and stop on invalid benchmark inputs"
)]
fn print_report(report: &AllReport, banded: &SizeBandedCorpus, timing: ReportTiming) {
    let ReportTiming {
        parse_secs,
        load_secs,
        repeat,
        parallel,
    } = timing;
    println!("=== throughput_by_class ===");
    println!();
    println!(
        "Corpus: {} docs across 4 bands; {} decode errors; {} I/O errors",
        banded.total_docs(),
        banded.decode_errors,
        banded.io_errors,
    );
    println!(
        "Wall:    load {load_secs:.2}s   parse {parse_secs:.2}s ({repeat} pass{plural})",
        plural = if repeat == 1 { "" } else { "es" }
    );
    if parallel {
        print_scaling(report, parse_secs, repeat);
    }
    println!();
    println!(
        "{:<13} {:>6} {:>13} {:>10} {:>10} {:>10} {:>10} {:>11} {:>10}",
        "band", "docs", "bytes", "MB/s", "p50 µs", "p90 µs", "p99 µs", "max ms", "ns/byte"
    );
    println!("{}", "-".repeat(106));
    for (slot, band) in SizeBand::ordered().into_iter().enumerate() {
        let r = &report.bands[slot];
        let docs = r.latencies_ns.len();
        if docs == 0 {
            println!(
                "{:<13} {:>6} {:>13} {:>10} {:>10} {:>10} {:>10} {:>11} {:>10}",
                band.label(),
                0,
                0,
                "-",
                "-",
                "-",
                "-",
                "-",
                "-"
            );
            continue;
        }
        let mut sorted = r.latencies_ns.clone();
        sorted.sort_unstable();
        let p50 = BandReport::quantile(&sorted, 0.5);
        let p90 = BandReport::quantile(&sorted, 0.9);
        let p99 = BandReport::quantile(&sorted, 0.99);
        let max_ns = *sorted.last().expect("non-empty");
        let total_bytes = r.total_bytes();
        let total_ns = r.total_ns();
        let mb_per_s = if total_ns == 0 {
            0.0
        } else {
            (report_number(total_bytes)) * NS_PER_S / (report_number(total_ns)) / (1024.0 * 1024.0)
        };
        let ns_per_byte = if total_bytes == 0 {
            0.0
        } else {
            (report_number(total_ns)) / (report_number(total_bytes))
        };
        println!(
            "{:<13} {:>6} {:>13} {:>10.1} {:>10.1} {:>10.1} {:>10.1} {:>11.2} {:>10.2}",
            band.label(),
            docs,
            total_bytes,
            mb_per_s,
            (report_number(p50)) / 1000.0,
            (report_number(p90)) / 1000.0,
            (report_number(p99)) / 1000.0,
            (report_number(max_ns)) / 1_000_000.0,
            ns_per_byte,
        );
    }
}

#[expect(
    clippy::print_stdout,
    reason = "benchmark commands report measurements and failures"
)]
fn print_scaling(report: &AllReport, parse_secs: f64, repeat: usize) {
    let serial_work_ns: u64 = report
        .bands
        .iter()
        .flat_map(|b| b.latencies_ns.iter().copied())
        .sum();
    let serial_work_secs = (report_number(serial_work_ns)) / NS_PER_S;
    let single_pass_secs = parse_secs / (report_number(repeat));
    let scaling = if single_pass_secs > 0.0 {
        serial_work_secs / single_pass_secs
    } else {
        0.0
    };
    let threads = rayon::current_num_threads();
    println!(
        "Parallel: {threads} threads   serial-work {serial_work_secs:.2}s   \
             concurrent-wall {single_pass_secs:.2}s   scaling {scaling:.2}× (ideal {threads}×)"
    );
}
