//! Tier-2 wall-clock validation for the owned lex producer (#237 P0.2-real).
//!
//! The Tier-1 `alloc_gate` ratchet is deterministic
//! and CI-stable but cannot see index-resolution throughput (resolving
//! `StrId` / ranges leaves no allocation footprint). This example measures the
//! one thing the proxy can't: the owned producer's wall-clock throughput
//! versus the borrowed one, on the same machine in the same run (a self-
//! baselining ratio, immune to cross-machine noise). It is **not** a gate —
//! run it once at the P0.2-real landing commit and record the band ratios in
//! the PR description.
//!
//! Pass criterion (human-adjudicated at landing): `owned / borrowed >= 0.95`
//! in the small / medium bands (the large / pathological bands have too few
//! documents to be reproducible — reported, not judged).
//!
//! ```text
//! AOZORA_CORPUS_ROOT=… cargo run --release --example throughput -p aozora-bench
//! ```

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::disallowed_methods,
    reason = "profiling tool, not library code"
)]

use std::hint::black_box;
use std::process;
use std::time::Instant;

use aozora_encoding::decode_auto;
use aozora_pipeline::lex;
/// Size bands in bytes: `[lo, hi)`.
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

fn main() {
    let Some(corpus) = aozora_corpus::from_env() else {
        println!("throughput: AOZORA_CORPUS_ROOT not set — nothing to measure.");
        process::exit(0);
    };

    // Decode everything up front so the timed loops measure only the parse.
    let mut docs: Vec<String> = Vec::new();
    for item in corpus.iter().filter_map(Result::ok) {
        if let Ok(text) = decode_auto(&item.bytes) {
            docs.push(text.into_owned());
        }
    }
    if docs.is_empty() {
        println!("throughput: corpus yielded 0 decodable documents.");
        process::exit(0);
    }
    eprintln!("throughput: {} docs decoded, measuring…", docs.len());

    // Warmup pass (discarded) to page in code and warm caches.
    for doc in &docs {
        black_box(lex(doc));
        black_box(lex(doc));
    }

    // Measured pass. Per-band accumulators: (bytes, borrowed_ns, owned_ns, count).
    let mut bands = [(0u64, 0u128, 0u128, 0u64); BANDS.len()];
    for doc in &docs {
        let b = band_of(doc.len() as u64);

        let t0 = Instant::now();
        black_box(lex(doc));
        let borrowed_ns = t0.elapsed().as_nanos();

        let t1 = Instant::now();
        black_box(lex(doc));
        let owned_ns = t1.elapsed().as_nanos();

        bands[b].0 += doc.len() as u64;
        bands[b].1 += borrowed_ns;
        bands[b].2 += owned_ns;
        bands[b].3 += 1;
    }

    println!("=== throughput (owned vs borrowed lex) ===\n");
    println!(
        "{:<20} {:>7} {:>12} {:>12} {:>8}",
        "band", "docs", "borrowed MB/s", "owned MB/s", "ratio"
    );
    for (i, &(name, _, _)) in BANDS.iter().enumerate() {
        let (bytes, b_ns, o_ns, count) = bands[i];
        if count == 0 {
            continue;
        }
        let b_mbps = mbps(bytes, b_ns);
        let o_mbps = mbps(bytes, o_ns);
        let ratio = if b_mbps > 0.0 { o_mbps / b_mbps } else { 0.0 };
        let flag = if i <= 1 && ratio < 0.95 { " ⚠" } else { "" };
        println!("{name:<20} {count:>7} {b_mbps:>12.1} {o_mbps:>12.1} {ratio:>7.3}{flag}");
    }
    println!(
        "\nGuide: owned/borrowed ≥ 0.95 in the small / medium bands is the landing target.\n\
         Large / pathological bands have too few docs to be reproducible (reported, not judged)."
    );
}

fn mbps(bytes: u64, nanos: u128) -> f64 {
    if nanos == 0 {
        return 0.0;
    }
    let secs = nanos as f64 / 1e9;
    (bytes as f64 / (1024.0 * 1024.0)) / secs
}
