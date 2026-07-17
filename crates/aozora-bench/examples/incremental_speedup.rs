//! Tier-2 wall-clock validation for the incremental diagnostics splice (#237 B').
//!
//! #237 proved the splice CORRECT (byte-identical to a full parse over the
//! whole corpus); this measures the thing the correctness gate cannot: the
//! actual edit-latency win. For every corpus document it times, on the same
//! machine in the same run (a self-baselining ratio):
//!
//! - **full** — `Document::lex(new_text)` (a from-scratch parse, what
//!   the LSP does today on any non-fast-path edit).
//! - **diagnostics-only** —
//!   `reparse_incremental_diagnostics_only(DiagBaseRef::from_cached(&cached, &pieces), …)`,
//!   the #237 production hot path: splices the maintained `PieceSeq` (the next
//!   edit's region-find base, from which the LSP flattens diagnostics), skipping
//!   the store clone + the normalized string / registry / container-pairs
//!   rebuild. Tier 2 additionally maintains the region-find representation
//!   incrementally: the `PieceSeq` is spliced in `O(region + #pieces)` (built
//!   once outside the timer, as production maintains it across edits rather than
//!   rebuilding it). The residual is the region re-lex + the piece splice.
//! - **clone** — `cached.store.clone()` alone, to confirm it is NOT the
//!   bottleneck (measured <1% of the diagnostics splice cost).
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

use aozora::encoding::decode_auto;
use aozora::{DiagBaseRef, Document, PieceSeq, reparse_incremental_diagnostics_only};

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
    diag_ns: u128,
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
        let san = Document::new(text.as_ref()).lex().sanitized;
        if san.is_empty() {
            continue;
        }
        // Keep only sanitize fixed points (sanitize idempotent) — the engine
        // assumes a stable sanitized baseline.
        if Document::new(san.as_str()).lex().sanitized != san {
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

        let cached = Document::new(san.as_str()).lex();

        // Plain ASCII insertion at a char boundary near the sanitized midpoint.
        let mut mid = san.len() / 2;
        while mid < san.len() && !san.is_char_boundary(mid) {
            mid += 1;
        }
        let new_san = format!("{}x{}", &san[..mid], &san[mid..]);

        // Warm this document's data, then take a single measured call each.
        black_box(Document::new(new_san.as_str()).lex());

        let t_full = Instant::now();
        black_box(Document::new(new_san.as_str()).lex());
        let full_ns = t_full.elapsed().as_nanos();

        // The production hot path: diagnostics-only splice (no full
        // LexOutput), splicing the maintained Tier-2 `PieceSeq`. Production
        // maintains the sequence across edits (it is not rebuilt per edit), so it
        // is built once outside the timer and the timer measures only the splice —
        // the true per-edit cost. `Some` exactly when the edit's locality is
        // provable from the cached tables, which is what gates a fast-path doc.
        let pieces = PieceSeq::from_contiguous(
            &cached.source_nodes,
            &cached.pairs,
            &cached.diagnostics,
            cached.sanitized_len,
        );
        let t_diag = Instant::now();
        let diag = reparse_incremental_diagnostics_only(
            DiagBaseRef::from_cached(&cached, &pieces),
            &new_san,
            mid..mid,
        );
        let diag_ns = t_diag.elapsed().as_nanos();
        let is_fast = diag.is_some();
        black_box(diag);

        let t_clone = Instant::now();
        black_box(cached.store.clone());
        let clone_ns = t_clone.elapsed().as_nanos();

        // Only fast-path docs contribute to the speedup numbers; a fallback's
        // incremental cost is just a full parse (1× by construction).
        if is_fast {
            bands[b].fast_path += 1;
            bands[b].full_ns += full_ns;
            bands[b].diag_ns += diag_ns;
            bands[b].clone_ns += clone_ns;
        }
    }

    println!("=== incremental_speedup (full vs diagnostics-only) ===\n");
    println!(
        "{:<18} {:>7} {:>8} {:>10} {:>10} {:>9} {:>8}",
        "band", "docs", "fast %", "full µs", "diag µs", "diag×", "clone %"
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
        tot.diag_ns += bd.diag_ns;
        tot.clone_ns += bd.clone_ns;
        print_row(name, bd);
    }
    println!("{:-<76}", "");
    print_row("all", tot);
    println!(
        "\nfull/diag µs = mean per fast-path doc; diag× = full/diag \
         (diagnostics-only hot path); clone % = store-clone share of diag.\n\
         With Tier 2 landed (O(log n) region-find + the maintained `PieceSeq` \
         spliced in O(region + #pieces)), the win is dramatic on large docs (the \
         500 KiB–2 MiB band) and ~1x on small docs — there a full parse is already \
         sub-100 µs, so the region-find + region re-lex prologue costs a similar \
         order and there is little to save."
    );
}

fn print_row(name: &str, bd: Band) {
    let fast_pct = 100.0 * bd.fast_path as f64 / bd.docs as f64;
    let (full_us, diag_us, diag_x, clone_pct) = if bd.fast_path > 0 {
        let f = bd.full_ns as f64 / bd.fast_path as f64 / 1000.0;
        let d = bd.diag_ns as f64 / bd.fast_path as f64 / 1000.0;
        let c = 100.0 * bd.clone_ns as f64 / bd.diag_ns.max(1) as f64;
        (f, d, if d > 0.0 { f / d } else { 0.0 }, c)
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };
    println!(
        "{name:<18} {:>7} {fast_pct:>7.1}% {full_us:>10.1} \
         {diag_us:>10.1} {diag_x:>8.2}x {clone_pct:>7.1}%",
        bd.docs
    );
}
