//! Bake-off bench: the production scanner vs the naive reference,
//! across four corpus shapes. Serves as the regression sentinel for
//! future scanner changes.
//!
//! ## Backends measured
//!
//! - **`naive`** — brute-force `classify`-per-byte reference. Slowest
//!   by design; the floor that anchors the speedup factor.
//! - **`production`** — [`aozora_scan::scan_offsets`], the shipping
//!   `aho-corasick`-backed packed matcher.
//!
//! ## Corpus shapes
//!
//! - **`plain_japanese`** — pure hiragana, no triggers. Tests the
//!   no-candidate fast path on the realistic byte distribution
//!   (~33 % of bytes are 0xE3 leading bytes).
//! - **`sparse_triggers`** — corpus median (~1 trigger per 200 B).
//! - **`dense_triggers`** — annotation-heavy editorial style (~1
//!   trigger per 8 B).
//! - **`ascii_text`** — pure ASCII English. Trigger-leading bytes
//!   are completely absent; tests the no-candidate fast path on a
//!   different byte distribution.
//!
//! Each sample is sized to 64 KiB — comfortably above L1 so the
//! bench measures sustained throughput, not L1 cache hits.

#![allow(
    clippy::cast_precision_loss,
    clippy::missing_panics_doc,
    reason = "bench code, not library"
)]

use aozora_scan::{NaiveScanner, scan_offsets};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

const TARGET_BYTES: usize = 64 * 1024;

fn build_plain_japanese(target: usize) -> String {
    let unit = "あいうえおかきくけこさしすせそ"; // 15 hiragana × 3 = 45 bytes
    repeat_to(unit, target)
}

fn build_sparse_triggers(target: usize) -> String {
    // ~1 trigger per ~200 bytes (corpus median density).
    let unit = "あいうえおかきくけこさしすせそ漢《かん》字。";
    repeat_to(unit, target)
}

fn build_dense_triggers(target: usize) -> String {
    // ~1 trigger per ~8 bytes (annotation-heavy editorial style).
    let unit = "｜青《あお》｜空《そら》｜山《やま》｜河《かわ》";
    repeat_to(unit, target)
}

fn build_ascii_text(target: usize) -> String {
    let unit = "the quick brown fox jumps over the lazy dog ";
    repeat_to(unit, target)
}

fn repeat_to(unit: &str, target: usize) -> String {
    let mut s = String::with_capacity(target + unit.len());
    while s.len() < target {
        s.push_str(unit);
    }
    s
}

fn bench_one(c: &mut Criterion, label: &str, sample: &str) {
    let mut g = c.benchmark_group(label);
    g.throughput(Throughput::Bytes(sample.len() as u64));

    g.bench_function("naive", |b| {
        b.iter(|| black_box(NaiveScanner.scan_offsets(black_box(sample))));
    });

    g.bench_function("production", |b| {
        b.iter(|| black_box(scan_offsets(black_box(sample))));
    });

    g.finish();
}

fn bench_scan_throughput(c: &mut Criterion) {
    let plain = build_plain_japanese(TARGET_BYTES);
    let sparse = build_sparse_triggers(TARGET_BYTES);
    let dense = build_dense_triggers(TARGET_BYTES);
    let ascii = build_ascii_text(TARGET_BYTES);

    bench_one(c, "plain_japanese", &plain);
    bench_one(c, "sparse_triggers", &sparse);
    bench_one(c, "dense_triggers", &dense);
    bench_one(c, "ascii_text", &ascii);
}

criterion_group!(bakeoff, bench_scan_throughput);
criterion_main!(bakeoff);
