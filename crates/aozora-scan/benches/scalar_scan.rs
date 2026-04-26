//! Throughput baseline for the scalar `memchr3`-driven trigger scanner.
//!
//! Each sample input mimics a band of the real-world Aozora corpus:
//!
//! - **`plain_japanese`**: pure hiragana/kanji, no triggers — exercises
//!   the all-skip fast path (memchr3 can sweep the whole buffer).
//! - **`sparse_triggers`**: ~1 trigger per 200 bytes (corpus median) —
//!   tests the precise-classify path on real-world density.
//! - **`dense_triggers`**: ~1 trigger per 8 bytes (annotation-heavy
//!   pathological docs like 鳥谷部春汀『明治人物月旦』).
//!
//! Throughput numbers reported via `criterion`'s `Throughput::Bytes`
//! so `cargo bench` prints MB/s alongside time.

#![allow(
    clippy::cast_precision_loss,
    clippy::missing_panics_doc,
    reason = "bench code, not library"
)]

#[cfg(target_arch = "x86_64")]
use aozora_scan::Avx2Scanner;
use aozora_scan::{ScalarScanner, TriggerScanner};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

fn build_plain_japanese(target_bytes: usize) -> String {
    // 36 bytes per cycle of "あいうえおかきくけこさしすせそ" (15 hiragana × 3 bytes).
    let unit = "あいうえおかきくけこさしすせそ";
    let unit_bytes = unit.len();
    let cycles = target_bytes.div_ceil(unit_bytes);
    let mut s = String::with_capacity(cycles * unit_bytes);
    for _ in 0..cycles {
        s.push_str(unit);
    }
    s
}

fn build_sparse_triggers(target_bytes: usize) -> String {
    // ~1 trigger per 200 bytes: 195 bytes of plain hiragana then a 3-byte
    // ruby-open marker, repeated.
    let plain = "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをんあいうえおか"; // 33 chars × 3 bytes = 99 bytes
    let unit = format!("{plain}{plain}《"); // 99 + 99 + 3 = 201 bytes
    let unit_bytes = unit.len();
    let cycles = target_bytes.div_ceil(unit_bytes);
    let mut s = String::with_capacity(cycles * unit_bytes);
    for _ in 0..cycles {
        s.push_str(&unit);
    }
    s
}

fn build_dense_triggers(target_bytes: usize) -> String {
    // 1 trigger per 8 bytes: 5 bytes plain + 3-byte trigger.
    // Use ASCII for the plain to make the spacing easy to reason about.
    let unit = "abcde《"; // 5 + 3 = 8 bytes
    let unit_bytes = unit.len();
    let cycles = target_bytes.div_ceil(unit_bytes);
    let mut s = String::with_capacity(cycles * unit_bytes);
    for _ in 0..cycles {
        s.push_str(unit);
    }
    s
}

fn bench_scan_throughput(c: &mut Criterion) {
    const TARGET: usize = 64 * 1024; // 64 KiB — comfortably above L1, fits in L2.
    let plain = build_plain_japanese(TARGET);
    let sparse = build_sparse_triggers(TARGET);
    let dense = build_dense_triggers(TARGET);

    for (label, sample) in [
        ("plain_japanese", &plain),
        ("sparse_triggers", &sparse),
        ("dense_triggers", &dense),
    ] {
        let mut g = c.benchmark_group(label);
        g.throughput(Throughput::Bytes(sample.len() as u64));
        g.bench_function("scalar", |b| {
            b.iter(|| {
                let offsets = ScalarScanner.scan_offsets(black_box(sample));
                black_box(offsets);
            });
        });
        #[cfg(target_arch = "x86_64")]
        if std::is_x86_feature_detected!("avx2") {
            g.bench_function("avx2", |b| {
                b.iter(|| {
                    let offsets = Avx2Scanner.scan_offsets(black_box(sample));
                    black_box(offsets);
                });
            });
        }
        g.finish();
    }
}

criterion_group!(scan_benches, bench_scan_throughput);
criterion_main!(scan_benches);
