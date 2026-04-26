//! Synthetic corpus benchmark — portable replacement for the
//! `crime_and_punishment` bench when `AOZORA_CORPUS_ROOT` is not set.
//!
//! Builds an aozora-shaped buffer at sizes spanning small editor
//! buffers (16 KiB) to large novels (3 MiB) so the criterion plot
//! shows the parse curve over realistic input sizes.

#![allow(clippy::missing_panics_doc, reason = "bench code, not library")]

use std::hint::black_box;

use aozora::Document;
use aozora_bench::build_synthetic_aozora;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

fn bench_synthetic_corpus(c: &mut Criterion) {
    let mut g = c.benchmark_group("synthetic_corpus");
    for size in [
        16 * 1024,       // 16 KiB — typical editor buffer
        128 * 1024,      // 128 KiB — short story
        512 * 1024,      // 512 KiB — long short story (above PARALLEL_THRESHOLD)
        2 * 1024 * 1024, // 2 MiB  — novel-scale
    ] {
        let buf = build_synthetic_aozora(size);
        g.throughput(Throughput::Bytes(buf.len() as u64));
        g.bench_with_input(BenchmarkId::from_parameter(buf.len()), &buf, |b, sample| {
            b.iter(|| {
                let doc = Document::new(black_box(sample.as_str()));
                let tree = doc.parse();
                black_box(tree);
            });
        });
    }
    g.finish();
}

criterion_group!(synthetic_benches, bench_synthetic_corpus);
criterion_main!(synthetic_benches);
