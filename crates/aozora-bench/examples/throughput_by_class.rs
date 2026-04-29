//! Per-size-class throughput report.
//!
//! Splits the corpus into four bands by post-decode UTF-8 byte length:
//!
//! - **Small**       `< 50 KiB`
//! - **Medium**      `50 KiB ..= 500 KiB`
//! - **Large**       `500 KiB ..= 2 MiB`
//! - **Pathological** `> 2 MiB`
//!
//! For each band, reports doc count, total bytes, single-thread parse
//! throughput (MB/s), p50 / p90 / p99 / max latency (ns), and per-byte
//! cost (ns/byte). The `phase_breakdown` probe lumps every doc together
//! and so over-weights the very few pathological documents that
//! dominate aggregate wall-clock; this probe makes the per-class
//! distribution visible.
//!
//! Reads `AOZORA_CORPUS_ROOT`. Optional `AOZORA_PROFILE_LIMIT=N` caps
//! the sweep. Optional `AOZORA_PROFILE_REPEAT=K` runs the parser
//! sweep K times back-to-back (load is paid once); useful when
//! profiling under `samply` because the longer parser-bound run
//! drowns out the corpus-decode fixed cost.
//!
//! ```text
//! AOZORA_CORPUS_ROOT=… cargo run --release --example throughput_by_class -p aozora-bench
//! ```
//!
//! Output is split into **load wall** (Shift-JIS decode + bucketing)
//! and **parse wall** (the actual `lex_into_arena` work). Earlier
//! versions reported a single "wall" that conflated both — when read
//! by a sampling profiler this caused the corpus-load syscalls
//! (`read` / `__nss_database_lookup` / `__memmove_avx_unaligned`) to
//! dominate the trace and bury the parser hot path.
//!
//! ## Parallel mode
//!
//! Set `AOZORA_PROFILE_PARALLEL=1` to fan per-doc parses across rayon's
//! work-stealing pool. Each task constructs its own [`Arena`], so
//! arenas remain `Send` (one per task) without breaking `bumpalo`'s
//! `!Sync` contract. Output gains a `[parallel: N threads]` header,
//! the parse wall reflects concurrent wall-clock (not the sum of
//! per-doc times), and a `scaling` line reports the effective speedup
//! vs the same per-doc work executed serially. Sequential remains the
//! default — per-doc latency numbers stay reproducible / variance-stable
//! and the sampling profiler attaches cleanly to a single-thread call
//! stack.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::disallowed_methods,
    reason = "profiling-example tool, not library code"
)]

use std::cell::RefCell;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::time::Instant;

use aozora_bench::{
    SizeBand, SizeBandedCorpus, archive_size_bands, corpus_size_bands_from_decoded,
    parallel_size_bands,
};
use aozora_corpus::{Archive, CorpusItem, FilesystemCorpus};
use aozora_encoding::decode_sjis;
use aozora_lex::lex_into_arena;
use aozora_syntax::borrowed::Arena;
use rayon::prelude::*;

// One Arena per worker thread, reused across the docs that worker
// processes. `Bump::reset()` between docs drops the prior parse's
// allocations without releasing the chunks — saving the per-doc
// `mmap` syscall a per-task `Arena::new()` would pay.
//
// Pre-size the thread-local arena at thread startup so the first
// few docs each worker sees don't pay the chunk-grow doubling tax
// (bumpalo's default initial chunk is
// 512 bytes; the corpus median doc needs ~50 KB of arena space, so
// without a hint the first ~7 docs each worker sees do
// 512 → 1 K → 2 K → 4 K → 8 K → 16 K → 32 K → 64 K growth). 256 KB
// covers >95 % of corpus docs in one chunk; the >2 MB outliers
// extend once and stay extended across resets, so the cost is paid
// once per worker per outlier-class instead of once per parse.
//
// `RefCell` matches `Arena`'s `!Sync` contract exactly (the rayon
// pool gives each worker its own thread-local cell, so the cell is
// never observed from a second thread). The borrow scope must close
// before the closure returns — which it does, because the closure
// drops `_out` immediately after timing.
const WORKER_ARENA_INITIAL_CAPACITY: usize = 256 * 1024;

thread_local! {
    static WORKER_ARENA: RefCell<Arena> = RefCell::new(Arena::with_capacity(WORKER_ARENA_INITIAL_CAPACITY));
}

const NS_PER_S: f64 = 1_000_000_000.0;

/// Whether to fan per-doc parses across rayon's work-stealing pool.
/// Opt-in via `AOZORA_PROFILE_PARALLEL=1` so the default sampling /
/// profiling workflow stays single-threaded and reproducible.
fn parallel_mode() -> bool {
    matches!(
        env::var("AOZORA_PROFILE_PARALLEL").ok().as_deref(),
        Some("1" | "true" | "yes")
    )
}

fn main() {
    let archive_path = env::var_os("AOZORA_CORPUS_ARCHIVE");
    let root = env::var_os("AOZORA_CORPUS_ROOT");
    if archive_path.is_none() && root.is_none() {
        eprintln!(
            "neither AOZORA_CORPUS_ARCHIVE nor AOZORA_CORPUS_ROOT is set; nothing to profile."
        );
        process::exit(2);
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

    // If AOZORA_CORPUS_ARCHIVE is set, load from a packed binary
    // archive (single fs::read of the whole archive + parallel iter
    // over its index) instead of walking 17 k small files. Skips
    // walkdir + per-file syscalls entirely; for the pre-decoded
    // UTF-8 + zstd variant also skips the per-doc decode.
    // AOZORA_CORPUS_ROOT remains the fallback when ARCHIVE is unset.
    let load = archive_path.as_ref().map_or_else(
        || {
            // Safe to unwrap: we returned exit(2) above if both were unset.
            let root = root.expect("AOZORA_CORPUS_ROOT must be set when ARCHIVE is not");
            let Ok(corpus) = FilesystemCorpus::new(PathBuf::from(&root)) else {
                eprintln!("AOZORA_CORPUS_ROOT is not a readable directory; nothing to profile.");
                process::exit(2);
            };

            // The serial path splits I/O + decode + bucketing into
            // four timers so each sub-phase's wall is visible. The
            // parallel path fans out via rayon (`parallel_size_bands`)
            // and overlaps read/decode, so the per-phase serial split
            // is suppressed there — the per-phase timers no longer
            // attribute meaningfully under concurrent dispatch.
            if parallel {
                LoadPhase::run_parallel(&corpus)
            } else {
                LoadPhase::run(&corpus, limit)
            }
        },
        |archive_path| LoadPhase::run_archive(Path::new(archive_path)),
    );

    eprintln!(
        "throughput_by_class: bucketed (small={}, medium={}, large={}, path={}, decode_err={})",
        load.banded.small.len(),
        load.banded.medium.len(),
        load.banded.large.len(),
        load.banded.pathological.len(),
        load.banded.decode_errors,
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
            split.sjis_bytes_total as f64 / 1_048_576.0,
        );
        eprintln!(
            "  ├─ decode  : {:>5.2}s ({:>4.1}%, {:.1} MB utf8 → {:.0} MB/s)",
            split.decode_secs,
            100.0 * split.decode_secs / load.total_secs,
            split.utf8_bytes_total as f64 / 1_048_576.0,
            (split.sjis_bytes_total as f64 / 1_048_576.0) / split.decode_secs.max(f64::EPSILON),
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

    let parse_start = Instant::now();
    let mut report = measure_all(&load.banded, parallel);
    for _ in 1..repeat {
        // Discard repeats — only the per-doc latencies of the final
        // pass are kept; earlier passes serve to warm caches and
        // (importantly) to give a sampling profiler more parser-bound
        // wall time to attach to.
        report = measure_all(&load.banded, parallel);
    }
    let parse_secs = parse_start.elapsed().as_secs_f64();

    print_report(
        &report,
        &load.banded,
        parse_secs,
        load.total_secs,
        repeat,
        parallel,
    );
}

/// Load-phase result — total wall time plus, in sequential mode, the
/// per-sub-phase split from [`LoadPhase::run`]. Parallel mode sets
/// `split = None` because rayon overlaps the read/decode sub-phases
/// and the per-phase serial timers no longer attribute meaningfully.
struct LoadPhase {
    banded: SizeBandedCorpus,
    total_secs: f64,
    split: Option<LoadSplit>,
}

/// Per-sub-phase numbers — only populated by [`LoadPhase::run`]
/// (sequential mode).
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

        // 1. walkdir — enumerate file paths only (no read).
        let walk_start = Instant::now();
        let paths: Vec<PathBuf> = corpus
            .walk_paths()
            .take(limit.unwrap_or(usize::MAX))
            .filter_map(Result::ok)
            .collect();
        let walkdir_secs = walk_start.elapsed().as_secs_f64();
        let path_count = paths.len();

        // 2. read — pull bytes for every path.
        let read_start = Instant::now();
        let items: Vec<CorpusItem> = paths
            .iter()
            .filter_map(|p| corpus.read_path(p).ok())
            .collect();
        let read_secs = read_start.elapsed().as_secs_f64();
        let sjis_bytes_total: u64 = items.iter().map(|it| it.bytes.len() as u64).sum();

        // 3. decode — SJIS → UTF-8, drop failures (counted in step 4).
        let decode_start = Instant::now();
        let mut decoded: Vec<(String, String)> = Vec::with_capacity(items.len());
        let mut decode_errors: usize = 0;
        for item in items {
            let label = item.label;
            let bytes = item.bytes;
            match decode_sjis(&bytes) {
                Ok(text) => decoded.push((label, text)),
                Err(_) => decode_errors += 1,
            }
        }
        let decode_secs = decode_start.elapsed().as_secs_f64();
        let utf8_bytes_total: u64 = decoded.iter().map(|(_, t)| t.len() as u64).sum();

        // 4. bucket — assign each (label, text) to its size band.
        let bucket_start = Instant::now();
        let mut banded = corpus_size_bands_from_decoded(decoded);
        banded.decode_errors = decode_errors;
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

    fn run_parallel(corpus: &FilesystemCorpus) -> Self {
        let total_start = Instant::now();
        let banded = parallel_size_bands(corpus);
        let total_secs = total_start.elapsed().as_secs_f64();
        Self {
            banded,
            total_secs,
            split: None,
        }
    }

    /// Load from a packed binary archive. One `fs::read` of
    /// the archive file, then parallel iter (decompress / decode /
    /// bucket) on the physical-core pool.
    fn run_archive(path: &Path) -> Self {
        let total_start = Instant::now();
        let archive =
            Archive::open(path).unwrap_or_else(|err| panic!("open {}: {err}", path.display()));
        eprintln!(
            "throughput_by_class: archive {} ({} entries, {}{}, {:.1} MB on disk)",
            path.display(),
            archive.len(),
            if archive.is_utf8() { "UTF8 " } else { "SJIS " },
            if archive.is_zstd() { "ZSTD" } else { "RAW" },
            fs::metadata(path).map_or(0, |m| m.len()) as f64 / 1_048_576.0,
        );
        let banded = archive_size_bands(&archive);
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
    /// Per-doc total parse latency (`lex_into_arena`) in ns.
    latencies_ns: Vec<u64>,
    /// Per-doc input size (post-decode UTF-8 bytes).
    sizes_bytes: Vec<u64>,
}

impl BandReport {
    fn total_bytes(&self) -> u64 {
        self.sizes_bytes.iter().sum()
    }
    fn total_ns(&self) -> u64 {
        self.latencies_ns.iter().sum()
    }
    fn quantile(sorted: &[u64], p: f64) -> u64 {
        if sorted.is_empty() {
            return 0;
        }
        let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
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

/// Measure one size-band. Each closure invocation borrows the
/// current worker thread's reusable [`Arena`] from [`WORKER_ARENA`]
/// and resets it before parsing — drops the prior parse's
/// allocations without paying the per-doc `mmap` syscall a fresh
/// `Arena::new()` would cost. Under `parallel`, rayon's work-stealing
/// pool gives each worker its own thread-local cell, so the
/// `RefCell` is never observed from a second thread (matches
/// `Arena`'s `!Sync` contract).
fn measure_band(docs: &[(String, String)], parallel: bool) -> BandReport {
    // Pre-size the per-thread arena to `source.len() * 4` before each
    // parse. The factor matches the production `Document::new` path
    // and covers borrowed-AST shape on every observed corpus doc.
    // When the worker's arena is already at least that large
    // (steady state after the first big doc), `reset_with_hint`
    // degrades to plain `reset()` — no syscall. The growth path
    // moves the chunk-extend `mmap` from inside the parse hot loop
    // to before it, removing a source of intra-parse latency variance
    // and pre-paying the `brk` cost that profiling identified as a
    // hot-path contributor.
    let measure = |text: &str| -> (u64, u64) {
        WORKER_ARENA.with(|cell| {
            let mut arena = cell.borrow_mut();
            arena.reset_with_hint(text.len().saturating_mul(4));
            let t = Instant::now();
            let _out = lex_into_arena(text, &arena);
            let ns = t.elapsed().as_nanos() as u64;
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

fn print_report(
    report: &AllReport,
    banded: &SizeBandedCorpus,
    parse_secs: f64,
    load_secs: f64,
    repeat: usize,
    parallel: bool,
) {
    println!("=== throughput_by_class ===");
    println!();
    println!(
        "Corpus: {} docs across 4 bands; {} decode errors",
        banded.total_docs(),
        banded.decode_errors,
    );
    println!(
        "Wall:    load {load_secs:.2}s   parse {parse_secs:.2}s ({repeat} pass{plural})",
        plural = if repeat == 1 { "" } else { "es" }
    );
    if parallel {
        // Sum of per-doc latencies = the work that would have been
        // done serially. Concurrent wall = the parse_secs we just
        // measured (one parse pass = one rayon par_iter). Their ratio
        // is the achieved scaling factor.
        let serial_work_ns: u64 = report
            .bands
            .iter()
            .flat_map(|b| b.latencies_ns.iter().copied())
            .sum();
        let serial_work_secs = (serial_work_ns as f64) / NS_PER_S;
        let single_pass_secs = parse_secs / (repeat as f64);
        let scaling = if single_pass_secs > 0.0 {
            serial_work_secs / single_pass_secs / (repeat as f64)
        } else {
            0.0
        };
        let threads = rayon::current_num_threads();
        println!(
            "Parallel: {threads} threads   serial-work {serial_work_secs:.2}s   \
             concurrent-wall {single_pass_secs:.2}s   scaling {scaling:.2}× (ideal {threads}×)"
        );
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
            (total_bytes as f64) * NS_PER_S / (total_ns as f64) / (1024.0 * 1024.0)
        };
        let ns_per_byte = if total_bytes == 0 {
            0.0
        } else {
            (total_ns as f64) / (total_bytes as f64)
        };
        println!(
            "{:<13} {:>6} {:>13} {:>10.1} {:>10.1} {:>10.1} {:>10.1} {:>11.2} {:>10.2}",
            band.label(),
            docs,
            total_bytes,
            mb_per_s,
            (p50 as f64) / 1000.0,
            (p90 as f64) / 1000.0,
            (p99 as f64) / 1000.0,
            (max_ns as f64) / 1_000_000.0,
            ns_per_byte,
        );
    }
}
