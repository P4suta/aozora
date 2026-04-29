//! Isolated Shift-JIS decode benchmark.
//!
//! Measures `aozora_encoding::decode_sjis` throughput in isolation
//! from filesystem I/O. The corpus is fully read into memory before
//! the timer starts, so neither `walkdir` traversal nor `fs::read`
//! syscalls pollute the per-band decode-MB/s number.
//!
//! Mirrors the `throughput_by_class.rs` env-var contract:
//!
//! - `AOZORA_CORPUS_ROOT` (required) — corpus root directory.
//! - `AOZORA_PROFILE_LIMIT=N` — cap the sweep to the first N docs.
//! - `AOZORA_PROFILE_PARALLEL=1` — also run a rayon-parallel decode
//!   pass so its delta against the sequential baseline is visible
//!   side-by-side.
//!
//! Output columns (one row per size band, plus an aggregate row):
//!
//! | docs | sjis MB | utf8 MB | seq MB/s | par MB/s | scale | p50 / p90 / p99 / max µs |
//!
//! `seq MB/s` and `par MB/s` use **sjis bytes in / decode wall** so
//! the number maps to "how many SJIS megabytes can the decoder
//! consume per second". The scale column is `par MB/s ÷ seq MB/s`,
//! the per-band parallel-efficiency check.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::too_many_lines,
    clippy::disallowed_methods,
    reason = "profiling-example tool, not library code"
)]

use std::env;
use std::path::PathBuf;
use std::process;
use std::time::Instant;

use aozora_corpus::{CorpusItem, FilesystemCorpus};
use aozora_encoding::decode_sjis;
use rayon::prelude::*;

const NS_PER_S: f64 = 1_000_000_000.0;

fn parallel_mode() -> bool {
    matches!(
        env::var("AOZORA_PROFILE_PARALLEL").ok().as_deref(),
        Some("1" | "true" | "yes")
    )
}

fn main() {
    let Some(root) = env::var_os("AOZORA_CORPUS_ROOT") else {
        eprintln!("AOZORA_CORPUS_ROOT not set; nothing to profile.");
        process::exit(2);
    };
    let Ok(corpus) = FilesystemCorpus::new(PathBuf::from(&root)) else {
        eprintln!("AOZORA_CORPUS_ROOT is not a readable directory.");
        process::exit(2);
    };

    let limit: Option<usize> = env::var("AOZORA_PROFILE_LIMIT")
        .ok()
        .and_then(|s| s.trim().parse().ok());
    let parallel = parallel_mode();

    eprintln!("decode_throughput: starting (limit = {limit:?}, parallel_pass = {parallel})");

    // Pre-load every CorpusItem into memory so I/O is fully amortised
    // before we start timing. This is the central discipline of an
    // isolated decode benchmark — the timer must wrap *only* the
    // decode work.
    let load_start = Instant::now();
    let paths: Vec<PathBuf> = corpus
        .walk_paths()
        .take(limit.unwrap_or(usize::MAX))
        .filter_map(Result::ok)
        .collect();
    let items: Vec<CorpusItem> = paths
        .iter()
        .filter_map(|p| corpus.read_path(p).ok())
        .collect();
    let load_secs = load_start.elapsed().as_secs_f64();
    eprintln!(
        "decode_throughput: loaded {} items ({:.1} MB sjis) in {:.2}s — timer starts now",
        items.len(),
        items.iter().map(|it| it.bytes.len()).sum::<usize>() as f64 / 1_048_576.0,
        load_secs,
    );

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
}

#[derive(Default, Clone)]
struct BandStats {
    docs: usize,
    sjis_bytes: u64,
    utf8_bytes: u64,
    seq_decode_ns: u64,
    par_decode_ns: u64,
    /// Sequential per-doc decode latencies; populated only when
    /// non-empty so the aggregate can compute quantiles across all
    /// bands without per-band noise.
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
    // Bucket by SJIS byte length — we don't have UTF-8 sizes yet
    // (that's what the decode produces) so use input length, which is
    // ~75 % of UTF-8 length on Aozora data and gives a stable
    // distribution.
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

fn measure_band(items: &[CorpusItem], indices: &[usize], parallel: bool) -> BandStats {
    let docs = indices.len();
    let sjis_bytes: u64 = indices.iter().map(|&i| items[i].bytes.len() as u64).sum();

    // Sequential pass — produces the per-doc latency distribution and
    // the seq decode wall.
    let mut utf8_bytes: u64 = 0;
    let mut latencies_ns: Vec<u64> = Vec::with_capacity(docs);
    let seq_start = Instant::now();
    for &i in indices {
        let t = Instant::now();
        match decode_sjis(&items[i].bytes) {
            Ok(text) => {
                utf8_bytes += text.len() as u64;
                latencies_ns.push(t.elapsed().as_nanos() as u64);
            }
            Err(_) => latencies_ns.push(t.elapsed().as_nanos() as u64),
        }
    }
    let seq_decode_ns = seq_start.elapsed().as_nanos() as u64;

    // Parallel pass (optional) — fans the same work across rayon. We
    // discard per-doc latencies for the parallel pass because the
    // wall time is what matters for scaling analysis.
    let par_decode_ns = if parallel {
        let par_start = Instant::now();
        indices.par_iter().for_each(|&i| {
            // Discard the result — only wall time is measured here;
            // correctness was already exercised by the sequential pass
            // that populated `latencies_ns` above.
            drop(decode_sjis(&items[i].bytes));
        });
        par_start.elapsed().as_nanos() as u64
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

fn print_row(label: &str, s: &BandStats) {
    let sjis_mb = s.sjis_bytes as f64 / 1_048_576.0;
    let utf8_mb = s.utf8_bytes as f64 / 1_048_576.0;
    let seq_secs = s.seq_decode_ns as f64 / NS_PER_S;
    let seq_mbs = sjis_mb / seq_secs.max(f64::EPSILON);

    let mut sorted = s.latencies_ns.clone();
    sorted.sort_unstable();
    let p = |q: f64| -> f64 {
        if sorted.is_empty() {
            0.0
        } else {
            let idx = ((sorted.len() as f64 - 1.0) * q).round() as usize;
            sorted[idx.min(sorted.len() - 1)] as f64 / 1_000.0
        }
    };

    if s.par_decode_ns > 0 {
        let par_secs = s.par_decode_ns as f64 / NS_PER_S;
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
