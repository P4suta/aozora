//! Compare scan-driven vs legacy phase 1 tokenisation.
//!
//! Measures the two implementations on three input bands so we can
//! see where the scan-driven path wins, ties, or loses. Inputs are
//! synthetic (corpus-free) so the bench is portable.

#![allow(clippy::missing_panics_doc, reason = "bench code")]

use std::hint::black_box;

use aozora_lex::tokenize_with_scan;
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

        g.bench_function("legacy_tokenize", |b| {
            b.iter(|| {
                let toks = aozora_lexer::tokenize(black_box(sample));
                black_box(toks);
            });
        });

        g.bench_function("scan_tokenize", |b| {
            b.iter(|| {
                let toks = tokenize_with_scan(black_box(sample));
                black_box(toks);
            });
        });

        g.finish();
    }
}

criterion_group!(tokenize_benches, bench_tokenize);
criterion_main!(tokenize_benches);
