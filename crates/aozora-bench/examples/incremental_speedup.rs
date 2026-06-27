//! Tier-2 wall-clock validation for the owned incremental splice (#237 B').
//!
//! #237 proved the splice CORRECT (byte-identical to a full parse over the
//! whole corpus); this measures the thing the correctness gate cannot: the
//! actual edit-latency win. For every corpus document it times, on the same
//! machine in the same run (a self-baselining ratio):
//!
//! - **full** — `Document::parse_owned(new_text)` (a from-scratch parse, what
//!   the LSP does today on any non-fast-path edit).
//! - **incremental** — `reparse_incremental_owned(&cached, new_text, edit)`
//!   (re-lex only the minimal region + splice the owned tables).
//! - **clone** — `cached.store.clone()` alone, to see how much of the
//!   incremental cost is the `O(doc)` store clone the splice does every edit.
//!
//! A single deterministic plain-`x` insertion near each document's sanitized
//! midpoint exercises the splice fast path (global-free docs) or the full-parse
//! fallback. Only fast-path documents (where the splice returns `Some`) are
//! timed for the speedup, so the ratio reflects the real win; the fast-path
//! rate is reported separately.
//!
//! Not a gate — run once and record the bands in the PR.
//!
//! ```text
//! AOZORA_CORPUS_ROOT=… cargo run --release --example incremental_speedup -p aozora-bench
//! ```

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::disallowed_methods,
    clippy::too_many_lines,
    reason = "profiling tool, not library code"
)]

use std::hint::black_box;
use std::process;
use std::time::Instant;

use aozora::{Document, reparse_incremental_owned};
use aozora_encoding::decode_auto;

/// Size bands in bytes (sanitized length): `[lo, hi)`.
const BANDS: &[(&str, u64, u64)] = &[
    ("< 50 KiB", 0, 50 * 1024),
    ("50 KiB – 500 KiB", 50 * 1024, 500 * 1024),
    ("500 KiB – 2 MiB", 500 * 1024, 2 * 1024 * 1024),
    ("> 2 MiB", 2 * 1024 * 1024, u64::MAX),
];

fn band_of(len: u64) -> usize {
    BANDS
        .iter()
        .position(|&(_, lo, hi)| len >= lo && len < hi)
        .unwrap_or(0)
}

/// Per-band accumulators.
#[derive(Default, Clone, Copy)]
struct Band {
    docs: u64,
    fast_path: u64,
    full_ns: u128,
    incr_ns: u128,
    clone_ns: u128,
}

fn main() {
    let Some(corpus) = aozora_corpus::from_env() else {
        println!("incremental_speedup: AOZORA_CORPUS_ROOT not set — nothing to measure.");
        process::exit(0);
    };

    // Decode + sanitize up front so the timed loop measures only the edit paths.
    // Each entry is the document's **sanitized** text (the engine's coordinate
    // space) that is itself a sanitize fixed point.
    let mut docs: Vec<String> = Vec::new();
    for item in corpus.iter().filter_map(Result::ok) {
        let Ok(text) = decode_auto(&item.bytes) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let san = Document::new(text.as_ref()).parse_owned().sanitized;
        if san.is_empty() {
            continue;
        }
        // Keep only sanitize fixed points (sanitize idempotent) — the engine
        // assumes a stable sanitized baseline.
        if Document::new(san.as_str()).parse_owned().sanitized != san {
            continue;
        }
        docs.push(san);
    }
    if docs.is_empty() {
        println!("incremental_speedup: corpus yielded 0 usable documents.");
        process::exit(0);
    }
    eprintln!("incremental_speedup: {} docs ready, measuring…", docs.len());

    let mut bands = [Band::default(); BANDS.len()];

    for san in &docs {
        let b = band_of(san.len() as u64);
        bands[b].docs += 1;

        let cached = Document::new(san.as_str()).parse_owned();

        // Plain ASCII insertion at a char boundary near the sanitized midpoint.
        let mut mid = san.len() / 2;
        while mid < san.len() && !san.is_char_boundary(mid) {
            mid += 1;
        }
        let new_san = format!("{}x{}", &san[..mid], &san[mid..]);

        // Warm this document's data, then take a single measured call each.
        black_box(Document::new(new_san.as_str()).parse_owned());

        let t_full = Instant::now();
        black_box(Document::new(new_san.as_str()).parse_owned());
        let full_ns = t_full.elapsed().as_nanos();

        let t_incr = Instant::now();
        let spliced = reparse_incremental_owned(&cached, &new_san, mid..mid);
        let incr_ns = t_incr.elapsed().as_nanos();
        let is_fast = spliced.is_some();
        black_box(spliced);

        let t_clone = Instant::now();
        black_box(cached.store.clone());
        let clone_ns = t_clone.elapsed().as_nanos();

        // Only fast-path docs contribute to the speedup numbers; a fallback's
        // "incremental" cost is just a full parse (1× by construction).
        if is_fast {
            bands[b].fast_path += 1;
            bands[b].full_ns += full_ns;
            bands[b].incr_ns += incr_ns;
            bands[b].clone_ns += clone_ns;
        }
    }

    println!("=== incremental_speedup (full vs owned incremental splice) ===\n");
    println!(
        "{:<18} {:>7} {:>9} {:>11} {:>11} {:>8} {:>10}",
        "band", "docs", "fast %", "full µs", "incr µs", "speedup", "clone %"
    );
    let mut tot = Band::default();
    for (i, &(name, _, _)) in BANDS.iter().enumerate() {
        let bd = bands[i];
        if bd.docs == 0 {
            continue;
        }
        tot.docs += bd.docs;
        tot.fast_path += bd.fast_path;
        tot.full_ns += bd.full_ns;
        tot.incr_ns += bd.incr_ns;
        tot.clone_ns += bd.clone_ns;
        print_row(name, bd);
    }
    println!("{:-<78}", "");
    print_row("all", tot);
    println!(
        "\nfull µs / incr µs = mean per fast-path doc; speedup = full / incr; \
         clone % = store-clone share of incremental.\n\
         Larger speedup is better; a high clone % means the O(doc) store clone \
         dominates (move-store optimisation candidate)."
    );
}

fn print_row(name: &str, bd: Band) {
    let fast_pct = 100.0 * bd.fast_path as f64 / bd.docs as f64;
    let (full_us, incr_us, speedup, clone_pct) = if bd.fast_path > 0 {
        let f = bd.full_ns as f64 / bd.fast_path as f64 / 1000.0;
        let i = bd.incr_ns as f64 / bd.fast_path as f64 / 1000.0;
        let c = 100.0 * bd.clone_ns as f64 / bd.incr_ns.max(1) as f64;
        (f, i, if i > 0.0 { f / i } else { 0.0 }, c)
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };
    println!(
        "{name:<18} {:>7} {fast_pct:>8.1}% {full_us:>11.1} {incr_us:>11.1} {speedup:>7.2}x {clone_pct:>9.1}%",
        bd.docs
    );
}
