//! Bake-off bench: every shipped [`TriggerScanner`] backend × four
//! corpus shapes. Serves as the regression sentinel for future
//! scanner changes.
//!
//! ## Backends measured
//!
//! - **`naive`** — brute-force PHF reference. Slowest but a useful
//!   floor that anchors the speedup factor on every other backend.
//! - **`teddy`** — Hyperscan Teddy via `aho_corasick::packed`.
//!   Production winner.
//! - **`structural_bitmap`** — simdjson-style two-byte (lead × middle)
//!   AVX2 bitmap. Production fallback when Teddy can't build.
//! - **`dfa`** — `regex_automata::dfa::dense::DFA::new_many` over
//!   the 11 trigger trigrams. SIMD-free universal fallback.
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

#[cfg(target_arch = "x86_64")]
use aozora_scan::StructuralBitmapScanner;
use aozora_scan::{DfaScanner, NaiveScanner, TeddyScanner, TriggerScanner};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

const TARGET_BYTES: usize = 64 * 1024;

fn build_plain_japanese(target: usize) -> String {
    let unit = "あいうえおかきくけこさしすせそ"; // 15 hiragana × 3 = 45 bytes
    repeat_to(unit, target)
}

fn build_sparse_triggers(target: usize) -> String {
    // 99 + 99 + 3 = 201 bytes per cycle (~1 trigger per 200 bytes).
    let plain = "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをんあいうえおか";
    let unit = format!("{plain}{plain}《");
    repeat_to(&unit, target)
}

fn build_dense_triggers(target: usize) -> String {
    let unit = "abcde《"; // 5 + 3 = 8 bytes
    repeat_to(unit, target)
}

fn build_ascii_text(target: usize) -> String {
    // English-shaped 60-byte unit. No trigger leading bytes, no
    // trigger middle bytes: every backend takes its no-candidate
    // fast path here.
    let unit = "The quick brown fox jumps over the lazy dog. ABCDE 12345.\n";
    repeat_to(unit, target)
}

fn repeat_to(unit: &str, target: usize) -> String {
    let cycles = target.div_ceil(unit.len());
    let mut s = String::with_capacity(cycles * unit.len());
    for _ in 0..cycles {
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
    if let Some(teddy) = TeddyScanner::new() {
        g.bench_function("teddy", |b| {
            b.iter(|| black_box(teddy.scan_offsets(black_box(sample))));
        });
    }
    #[cfg(target_arch = "x86_64")]
    if std::is_x86_feature_detected!("avx2") {
        g.bench_function("structural_bitmap", |b| {
            b.iter(|| black_box(StructuralBitmapScanner.scan_offsets(black_box(sample))));
        });
    }
    let dfa = DfaScanner::new();
    g.bench_function("dfa", |b| {
        b.iter(|| black_box(dfa.scan_offsets(black_box(sample))));
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
