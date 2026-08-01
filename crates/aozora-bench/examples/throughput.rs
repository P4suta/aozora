//! Per-size-band parser throughput probe.
//!
//! The Tier-1 `alloc_gate` ratchet is deterministic
//! and CI-stable but cannot see index-resolution throughput (resolving
//! `StrId` / ranges leaves no allocation footprint). This example measures the
//! parser's current wall-clock throughput. It is a diagnostic, not a gate;
//! compare results only under the same machine and build configuration.
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

use aozora::decode_auto;
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
    let mut io_errors = 0;
    let mut decode_errors = 0;
    for item in corpus.iter() {
        match item {
            Ok(item) => match decode_auto(&item.bytes) {
                Ok(text) => docs.push(text.into_owned()),
                Err(_) => decode_errors += 1,
            },
            Err(_) => io_errors += 1,
        }
    }
    report_load_errors(io_errors, decode_errors);
    if docs.is_empty() {
        println!("throughput: corpus yielded 0 decodable documents.");
        process::exit(0);
    }
    eprintln!("throughput: {} docs decoded, measuring…", docs.len());

    // Warmup pass (discarded) to page in code and warm caches.
    for doc in &docs {
        black_box(
            aozora::parse(doc.as_str())
                .expect("source fits parser span limit")
                .snapshot(),
        );
    }

    let mut bands = [(0u64, 0u128, 0u64); BANDS.len()];
    for doc in &docs {
        let b = band_of(doc.len() as u64);

        let t0 = Instant::now();
        black_box(
            aozora::parse(doc.as_str())
                .expect("source fits parser span limit")
                .snapshot(),
        );
        let parse_ns = t0.elapsed().as_nanos();

        bands[b].0 += doc.len() as u64;
        bands[b].1 += parse_ns;
        bands[b].2 += 1;
    }

    println!("=== parse throughput ===\n");
    println!("{:<20} {:>7} {:>12}", "band", "docs", "MB/s");
    for (i, &(name, _, _)) in BANDS.iter().enumerate() {
        let (bytes, parse_ns, count) = bands[i];
        if count == 0 {
            continue;
        }
        let throughput = mbps(bytes, parse_ns);
        println!("{name:<20} {count:>7} {throughput:>12.1}");
    }
}

fn report_load_errors(io_errors: usize, decode_errors: usize) {
    if io_errors != 0 {
        eprintln!(
            "throughput: refusing a partial-corpus measurement after {io_errors} I/O error(s)"
        );
        process::exit(2);
    }
    if decode_errors != 0 {
        eprintln!("throughput: skipped {decode_errors} undecodable document(s)");
    }
}

fn mbps(bytes: u64, nanos: u128) -> f64 {
    if nanos == 0 {
        return 0.0;
    }
    let secs = nanos as f64 / 1e9;
    (bytes as f64 / (1024.0 * 1024.0)) / secs
}
