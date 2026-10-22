//! Measures decoding throughput by document size after loading corpus bytes into memory.

mod support;

use support::report_number;

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use aozora::decode_sjis;
use aozora_corpus::{CorpusItem, FilesystemCorpus};
use rayon::prelude::*;

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
    clippy::print_stdout,
    reason = "benchmark commands report measurements and failures"
)]
fn run() -> Result<(), ExitCode> {
    let Some(root) = env::var_os("AOZORA_CORPUS_ROOT") else {
        eprintln!("AOZORA_CORPUS_ROOT not set; nothing to profile.");
        return Err(ExitCode::from(2));
    };
    let Ok(corpus) = FilesystemCorpus::new(PathBuf::from(&root)) else {
        eprintln!("AOZORA_CORPUS_ROOT is not a readable directory.");
        return Err(ExitCode::from(2));
    };

    let limit: Option<usize> = env::var("AOZORA_PROFILE_LIMIT")
        .ok()
        .and_then(|s| s.trim().parse().ok());
    let parallel = parallel_mode();

    eprintln!("decode_throughput: starting (limit = {limit:?}, parallel_pass = {parallel})");

    let load_start = Instant::now();
    let mut paths = Vec::new();
    let mut io_errors = 0;
    for path in corpus.walk_paths().take(limit.unwrap_or(usize::MAX)) {
        match path {
            Ok(path) => paths.push(path),
            Err(_) => io_errors += 1,
        }
    }
    let mut items = Vec::with_capacity(paths.len());
    for path in &paths {
        match corpus.read_path(path) {
            Ok(item) => items.push(item),
            Err(_) => io_errors += 1,
        }
    }
    let load_secs = load_start.elapsed().as_secs_f64();
    eprintln!(
        "decode_throughput: loaded {} items ({:.1} MB sjis) in {:.2}s — timer starts now",
        items.len(),
        report_number(items.iter().map(|it| it.bytes.len()).sum::<usize>()) / 1_048_576.0,
        load_secs,
    );
    if io_errors != 0 {
        eprintln!(
            "decode_throughput: refusing a partial-corpus measurement after {io_errors} I/O error(s)"
        );
        return Err(ExitCode::from(2));
    }

    let bands = bucket_for_decode(&items);
    let band_labels = ["<50KB", "50KB-500KB", "500KB-2MB", ">2MB"];

    println!();
    print_header(parallel);

    let mut agg = BandStats::default();
    for (label, indices) in band_labels.iter().zip(bands.iter()) {
        let stats = measure_band(&items, indices, parallel);
        agg = agg.combine(&stats);
        print_row(label, &stats);
    }
    println!("  {}", "─".repeat(106));
    print_row("ALL", &agg);
    Ok(())
}

#[derive(Default, Clone)]
struct BandStats {
    docs: usize,
    sjis_bytes: u64,
    utf8_bytes: u64,
    seq_decode_ns: u64,
    par_decode_ns: u64,
    latencies_ns: Vec<u64>,
}

impl BandStats {
    fn combine(&self, other: &Self) -> Self {
        let mut latencies = self.latencies_ns.clone();
        latencies.extend_from_slice(&other.latencies_ns);
        Self {
            docs: self.docs + other.docs,
            sjis_bytes: self.sjis_bytes + other.sjis_bytes,
            utf8_bytes: self.utf8_bytes + other.utf8_bytes,
            seq_decode_ns: self.seq_decode_ns + other.seq_decode_ns,
            par_decode_ns: self.par_decode_ns + other.par_decode_ns,
            latencies_ns: latencies,
        }
    }
}

fn bucket_for_decode(items: &[CorpusItem]) -> [Vec<usize>; 4] {
    const SMALL_MAX: usize = 50 * 1024;
    const MEDIUM_MAX: usize = 500 * 1024;
    const LARGE_MAX: usize = 2 * 1024 * 1024;

    let mut bands: [Vec<usize>; 4] = Default::default();
    for (i, item) in items.iter().enumerate() {
        let n = item.bytes.len();
        let slot = if n < SMALL_MAX {
            0
        } else if n < MEDIUM_MAX {
            1
        } else if n < LARGE_MAX {
            2
        } else {
            3
        };
        bands[slot].push(i);
    }
    bands
}

#[expect(
    clippy::expect_used,
    reason = "benchmark inputs must meet parser and numeric preconditions"
)]
fn measure_band(items: &[CorpusItem], indices: &[usize], parallel: bool) -> BandStats {
    let docs = indices.len();
    let sjis_bytes: u64 = indices.iter().map(|&i| items[i].bytes.len() as u64).sum();

    let mut utf8_bytes: u64 = 0;
    let mut latencies_ns: Vec<u64> = Vec::with_capacity(docs);
    let seq_start = Instant::now();
    for &i in indices {
        let t = Instant::now();
        match decode_sjis(&items[i].bytes) {
            Ok(text) => {
                utf8_bytes += text.len() as u64;
                latencies_ns.push(
                    u64::try_from(t.elapsed().as_nanos()).expect("elapsed nanoseconds fit u64"),
                );
            }
            Err(_) => latencies_ns
                .push(u64::try_from(t.elapsed().as_nanos()).expect("elapsed nanoseconds fit u64")),
        }
    }
    let seq_decode_ns =
        u64::try_from(seq_start.elapsed().as_nanos()).expect("elapsed nanoseconds fit u64");

    let par_decode_ns = if parallel {
        let par_start = Instant::now();
        indices.par_iter().for_each(|&i| {
            drop(decode_sjis(&items[i].bytes));
        });
        u64::try_from(par_start.elapsed().as_nanos()).expect("elapsed nanoseconds fit u64")
    } else {
        0
    };

    BandStats {
        docs,
        sjis_bytes,
        utf8_bytes,
        seq_decode_ns,
        par_decode_ns,
        latencies_ns,
    }
}

#[expect(
    clippy::print_stdout,
    reason = "benchmark commands report measurements and failures"
)]
fn print_header(parallel: bool) {
    if parallel {
        println!(
            "  {:<14} {:>6} {:>10} {:>10} {:>10} {:>10} {:>7} {:>9} {:>9} {:>9} {:>9}",
            "band",
            "docs",
            "sjis MB",
            "utf8 MB",
            "seq MB/s",
            "par MB/s",
            "scale",
            "p50 µs",
            "p90 µs",
            "p99 µs",
            "max µs",
        );
    } else {
        println!(
            "  {:<14} {:>6} {:>10} {:>10} {:>10} {:>9} {:>9} {:>9} {:>9}",
            "band",
            "docs",
            "sjis MB",
            "utf8 MB",
            "seq MB/s",
            "p50 µs",
            "p90 µs",
            "p99 µs",
            "max µs",
        );
    }
    println!("  {}", "─".repeat(106));
}

#[expect(
    clippy::expect_used,
    clippy::print_stdout,
    reason = "benchmark commands report measurements and stop on invalid benchmark inputs"
)]
fn print_row(label: &str, s: &BandStats) {
    let sjis_mb = report_number(s.sjis_bytes) / 1_048_576.0;
    let utf8_mb = report_number(s.utf8_bytes) / 1_048_576.0;
    let seq_secs = report_number(s.seq_decode_ns) / NS_PER_S;
    let seq_mbs = sjis_mb / seq_secs.max(f64::EPSILON);

    let mut sorted = s.latencies_ns.clone();
    sorted.sort_unstable();
    let p = |q: f64| -> f64 {
        if sorted.is_empty() {
            0.0
        } else {
            let idx = ((report_number(sorted.len()) - 1.0) * q)
                .round()
                .to_string()
                .parse::<usize>()
                .expect("quantile index fits usize");
            report_number(sorted[idx.min(sorted.len() - 1)]) / 1_000.0
        }
    };

    if s.par_decode_ns > 0 {
        let par_secs = report_number(s.par_decode_ns) / NS_PER_S;
        let par_mbs = sjis_mb / par_secs.max(f64::EPSILON);
        let scale = par_mbs / seq_mbs.max(f64::EPSILON);
        println!(
            "  {:<14} {:>6} {:>10.2} {:>10.2} {:>10.1} {:>10.1} {:>6.2}× {:>9.1} {:>9.1} {:>9.1} {:>9.1}",
            label,
            s.docs,
            sjis_mb,
            utf8_mb,
            seq_mbs,
            par_mbs,
            scale,
            p(0.5),
            p(0.9),
            p(0.99),
            p(1.0),
        );
    } else {
        println!(
            "  {:<14} {:>6} {:>10.2} {:>10.2} {:>10.1} {:>9.1} {:>9.1} {:>9.1} {:>9.1}",
            label,
            s.docs,
            sjis_mb,
            utf8_mb,
            seq_mbs,
            p(0.5),
            p(0.9),
            p(0.99),
            p(1.0),
        );
    }
}
