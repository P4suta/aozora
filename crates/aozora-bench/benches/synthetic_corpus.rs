//! Benchmarks parser throughput on generated Aozora source.

use std::hint::black_box;

use aozora_bench::build_synthetic_aozora;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

#[expect(
    clippy::expect_used,
    reason = "benchmark inputs must meet parser and numeric preconditions"
)]
fn bench_synthetic_corpus(c: &mut Criterion) {
    let mut g = c.benchmark_group("synthetic_corpus");
    for size in [16 * 1024, 128 * 1024, 512 * 1024, 2 * 1024 * 1024] {
        let buf = build_synthetic_aozora(size);
        g.throughput(Throughput::Bytes(buf.len() as u64));
        g.bench_with_input(BenchmarkId::from_parameter(buf.len()), &buf, |b, sample| {
            b.iter(|| {
                let doc = aozora::parse(black_box(sample.as_str()))
                    .expect("source fits parser span limit");
                let tree = doc.snapshot();
                black_box(tree);
            });
        });
    }
    g.finish();
}

criterion_group!(synthetic_benches, bench_synthetic_corpus);
criterion_main!(synthetic_benches);
