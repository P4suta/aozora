//! Phase 1 throughput sentinel.
//!
//! Single-shape throughput regression gate over `aozora_lexer::tokenize`
//! (the SIMD-scan tokeniser). Three input bands cover the corpus
//! distribution.

#![allow(clippy::missing_panics_doc, reason = "bench code")]

use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};

fn build_plain(size: usize) -> String {
    let unit = "あいうえおかきくけこさしすせそ";
    let cycles = size.div_ceil(unit.len());
    unit.repeat(cycles)
}

fn build_sparse(size: usize) -> String {
    // ~1 trigger per 200 bytes (corpus median).
    let plain = "あいうえおかきくけこさしすせそたちつてとなにぬねのはひふへほまみむめもやゆよらりるれろわをんあいうえおか";
    let unit = format!("{plain}{plain}《");
    let cycles = size.div_ceil(unit.len());
    unit.repeat(cycles)
}

fn build_dense(size: usize) -> String {
    // ~1 trigger per 8 bytes (annotation-heavy pathological).
    let unit = "abcde《";
    let cycles = size.div_ceil(unit.len());
    unit.repeat(cycles)
}

fn bench_tokenize(c: &mut Criterion) {
    const SIZE: usize = 64 * 1024;
    let plain = build_plain(SIZE);
    let sparse = build_sparse(SIZE);
    let dense = build_dense(SIZE);

    for (label, sample) in [("plain", &plain), ("sparse", &sparse), ("dense", &dense)] {
        let mut g = c.benchmark_group(label);
        g.throughput(Throughput::Bytes(sample.len() as u64));

        g.bench_function("tokenize", |b| {
            b.iter(|| {
                let toks: Vec<_> = aozora_lexer::tokenize(black_box(sample)).collect();
                black_box(toks);
            });
        });

        g.finish();
    }
}

criterion_group!(tokenize_benches, bench_tokenize);
criterion_main!(tokenize_benches);
