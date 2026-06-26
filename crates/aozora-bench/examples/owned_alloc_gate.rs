//! Deterministic allocation-pressure ratchet for the owned lex producer
//! (`lex_owned` / `Document::parse_owned`), the #237 P0.2-real perf gate.
//!
//! The owned producer replaces the borrowed pipeline's single bumpalo arena
//! with owned `Vec` / `String` storage (`NodeStore`, `StrInterner`). The worry
//! the gate guards is that many small `Vec` pushes / re-grows inflate
//! `malloc` traffic versus the bump allocator. Allocation *count* and *bytes*
//! are a pure function of the input — unlike wall-clock they gate identically
//! on a laptop and a noisy CI runner — so they make a stable ratchet.
//!
//! For every corpus document this measures, via dhat's [`dhat::HeapStats`]
//! around `lex_owned` only, the owned-path allocation delta (transient arena
//! chunks + owned storage). Two normalized metrics are gated against a
//! committed baseline at `corpus/owned-alloc-baseline.json`, mirroring
//! `xtask corpus audit-gate`:
//!
//! - `alloc_blocks_per_file`        = Σ Δblocks / files   (malloc-count pressure)
//! - `alloc_bytes_per_source_byte`  = Σ Δbytes  / Σ src   (volume amplification)
//!
//! A structural-parity check (owned vs borrowed registry / source-node / pair
//! counts per document) is the correctness floor under the proxy: if the owned
//! mirror started producing a *different amount* of data, the alloc baseline
//! would be measuring the wrong thing. (Byte-identity itself is proven by the
//! `owned_serialize_gate` / `owned_html_gate` corpus gates.)
//!
//! ```text
//! AOZORA_CORPUS_ROOT=… cargo run --release --example owned_alloc_gate -p aozora-bench \
//!     -- --baseline corpus/owned-alloc-baseline.json [--root DIR] [--update] [--tolerance 0.03]
//! ```
//!
//! Exit codes: `0` pass (or no corpus → skip), `1` over budget, `2` usage /
//! parity error.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    clippy::disallowed_methods,
    reason = "profiling-gate tool, not library code"
)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

use aozora_corpus::{CorpusSource, FilesystemCorpus};
use aozora_encoding::decode_auto;
use aozora_pipeline::{lex, lex_owned};
use aozora_syntax::borrowed::Arena;
use dhat::{HeapStats, Profiler};
use serde_json::{Value, from_str, json, to_string_pretty};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

/// Default per-metric headroom over baseline. Wider than `audit-gate`'s 1%
/// because the CI corpus checks out `P4suta/aozorabunko_text` at unpinned HEAD
/// — the doc set drifts run-to-run, and 3% absorbs the resulting mean shift
/// once the per-file / per-source-byte normalization has absorbed count drift.
const DEFAULT_TOLERANCE: f64 = 0.03;

struct Args {
    baseline: PathBuf,
    root: Option<PathBuf>,
    update: bool,
    tolerance: f64,
    limit: Option<usize>,
}

fn parse_args() -> Args {
    let mut baseline = None;
    let mut root = None;
    let mut update = false;
    let mut tolerance = DEFAULT_TOLERANCE;
    let mut limit = None;
    let mut it = env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--baseline" => baseline = it.next().map(PathBuf::from),
            "--root" => root = it.next().map(PathBuf::from),
            "--update" => update = true,
            "--tolerance" => {
                tolerance = it
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(DEFAULT_TOLERANCE);
            }
            "--limit" => limit = it.next().and_then(|s| s.parse().ok()),
            other => {
                eprintln!("owned_alloc_gate: unknown argument {other:?}");
                process::exit(2);
            }
        }
    }
    let Some(baseline) = baseline else {
        eprintln!("owned_alloc_gate: --baseline <path> is required");
        process::exit(2);
    };
    Args {
        baseline,
        root,
        update,
        tolerance,
        limit,
    }
}

/// Resolve the corpus: `--root` wins, else `AOZORA_CORPUS_ROOT`, else skip
/// cleanly (exit 0) so the gate is a no-op on corpus-less hosts, exactly like
/// `audit-gate`.
fn resolve_corpus(root: Option<&Path>) -> Box<dyn CorpusSource> {
    if let Some(root) = root {
        return match FilesystemCorpus::new(root.to_path_buf()) {
            Ok(c) => Box::new(c),
            Err(e) => {
                eprintln!(
                    "owned_alloc_gate: --root {} not usable: {e}",
                    root.display()
                );
                process::exit(2);
            }
        };
    }
    if let Some(c) = aozora_corpus::from_env() {
        return c;
    }
    println!("owned_alloc_gate: AOZORA_CORPUS_ROOT not set — skipped (no corpus).");
    process::exit(0);
}

/// Per-corpus owned-allocation totals.
#[derive(Default)]
struct Totals {
    files: u64,
    source_bytes: u64,
    alloc_blocks: u64,
    alloc_bytes: u64,
    decode_errors: u64,
}

impl Totals {
    fn blocks_per_file(&self) -> f64 {
        if self.files == 0 {
            0.0
        } else {
            self.alloc_blocks as f64 / self.files as f64
        }
    }

    fn bytes_per_source_byte(&self) -> f64 {
        if self.source_bytes == 0 {
            0.0
        } else {
            self.alloc_bytes as f64 / self.source_bytes as f64
        }
    }
}

fn main() {
    let args = parse_args();

    let corpus = resolve_corpus(args.root.as_deref());

    // Testing mode lets `HeapStats::get` read live cumulative counters.
    let _profiler = Profiler::builder().testing().build();

    let mut totals = Totals::default();
    for item in corpus.iter().filter_map(Result::ok) {
        if let Some(limit) = args.limit
            && totals.files >= limit as u64
        {
            break;
        }
        let Ok(text) = decode_auto(&item.bytes) else {
            totals.decode_errors += 1;
            continue;
        };

        // Borrowed parity reference — built *before* the measured window so its
        // arena allocations are not attributed to the owned producer.
        let arena_b = Arena::new();
        let borrowed = lex(&text, &arena_b);
        let b_registry = borrowed.registry.len();
        let b_source_nodes = borrowed.source_nodes.len();
        let b_pairs = borrowed.pairs.len();
        let b_container_pairs = borrowed.container_pairs.len();
        drop(borrowed);
        drop(arena_b);

        // Measured window: only the owned producer's allocations (transient
        // arena chunks + owned storage) land in the delta.
        let before = HeapStats::get();
        let arena_o = Arena::new();
        let owned = lex_owned(&text, &arena_o);
        let after = HeapStats::get();
        totals.alloc_blocks += after.total_blocks - before.total_blocks;
        totals.alloc_bytes += after.total_bytes - before.total_bytes;
        totals.files += 1;
        totals.source_bytes += text.len() as u64;

        // Structural parity floor (reads only, post-measurement).
        assert_eq!(
            owned.registry.len(),
            b_registry,
            "owned/borrowed registry length diverged for {}",
            item.label
        );
        assert_eq!(
            owned.source_nodes.len(),
            b_source_nodes,
            "owned/borrowed source_nodes length diverged for {}",
            item.label
        );
        assert_eq!(
            owned.pairs.len(),
            b_pairs,
            "owned/borrowed pairs length diverged for {}",
            item.label
        );
        assert_eq!(
            owned.container_pairs.len(),
            b_container_pairs,
            "owned/borrowed container_pairs length diverged for {}",
            item.label
        );
    }

    if totals.files == 0 {
        println!("owned_alloc_gate: corpus yielded 0 decodable documents — skipped.");
        process::exit(0);
    }

    let blocks_per_file = totals.blocks_per_file();
    let bytes_per_source_byte = totals.bytes_per_source_byte();
    println!(
        "owned_alloc_gate: {} files, {} decode errors",
        totals.files, totals.decode_errors
    );
    println!("  alloc_blocks_per_file       = {blocks_per_file:.4}");
    println!("  alloc_bytes_per_source_byte = {bytes_per_source_byte:.4}");

    if args.update {
        write_baseline(&args, &totals, blocks_per_file, bytes_per_source_byte);
        println!(
            "owned_alloc_gate: baseline written to {}",
            args.baseline.display()
        );
        return;
    }

    let baseline = read_baseline(&args.baseline);
    let tol = baseline
        .tolerance
        .filter(|t| t.is_finite() && *t >= 0.0)
        .unwrap_or(args.tolerance);
    let mut failed = false;
    failed |= check_metric(
        "alloc_blocks_per_file",
        blocks_per_file,
        baseline.blocks_per_file,
        tol,
    );
    failed |= check_metric(
        "alloc_bytes_per_source_byte",
        bytes_per_source_byte,
        baseline.bytes_per_source_byte,
        tol,
    );

    if failed {
        eprintln!(
            "owned_alloc_gate: FAIL — owned allocation pressure regressed beyond +{:.1}%.\n  \
             Re-baseline with `just owned-alloc-gate-update` only if the increase is intended,\n  \
             and attach a `just owned-throughput` run showing wall-clock is within budget.",
            tol * 100.0
        );
        process::exit(1);
    }
    println!(
        "owned_alloc_gate: PASS (within +{:.1}% of baseline).",
        tol * 100.0
    );
}

/// Pass iff `current <= baseline * (1 + tolerance)`. A missing baseline metric
/// (NaN) is treated as a hard fail so a malformed baseline cannot pass blindly.
fn check_metric(name: &str, current: f64, baseline: f64, tolerance: f64) -> bool {
    if !baseline.is_finite() {
        eprintln!("owned_alloc_gate: baseline metric {name} missing/invalid");
        return true;
    }
    let allowed = baseline * (1.0 + tolerance);
    if current > allowed {
        eprintln!(
            "  {name}: {current:.4} > allowed {allowed:.4} (baseline {baseline:.4} +{:.1}%)",
            tolerance * 100.0
        );
        true
    } else {
        false
    }
}

struct Baseline {
    blocks_per_file: f64,
    bytes_per_source_byte: f64,
    tolerance: Option<f64>,
}

fn read_baseline(path: &Path) -> Baseline {
    let Ok(text) = fs::read_to_string(path) else {
        eprintln!("owned_alloc_gate: cannot read baseline {}", path.display());
        process::exit(2);
    };
    let Ok(v) = from_str::<Value>(&text) else {
        eprintln!(
            "owned_alloc_gate: baseline {} is not valid JSON",
            path.display()
        );
        process::exit(2);
    };
    Baseline {
        blocks_per_file: v["alloc_blocks_per_file"].as_f64().unwrap_or(f64::NAN),
        bytes_per_source_byte: v["alloc_bytes_per_source_byte"]
            .as_f64()
            .unwrap_or(f64::NAN),
        tolerance: v["tolerance"].as_f64(),
    }
}

fn write_baseline(args: &Args, totals: &Totals, blocks_per_file: f64, bytes_per_source_byte: f64) {
    let json = json!({
        "tolerance": args.tolerance,
        "files_analyzed": totals.files,
        "source_bytes_total": totals.source_bytes,
        "owned_alloc_blocks_total": totals.alloc_blocks,
        "owned_alloc_bytes_total": totals.alloc_bytes,
        "alloc_blocks_per_file": round4(blocks_per_file),
        "alloc_bytes_per_source_byte": round4(bytes_per_source_byte),
        "note": "owned lex producer (#237 P0.2-real) allocation-pressure ratchet; \
                 regenerate with `just owned-alloc-gate-update`. Metrics normalized \
                 per-file / per-source-byte so corpus drift does not trip the gate.",
    });
    let text = to_string_pretty(&json).expect("json serialize is infallible");
    if let Err(e) = fs::write(&args.baseline, format!("{text}\n")) {
        eprintln!(
            "owned_alloc_gate: cannot write baseline {}: {e}",
            args.baseline.display()
        );
        process::exit(2);
    }
}

/// Round to 4 decimals so the committed baseline is stable and reviewable.
fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}
