//! Criterion bench: intra-document parallel parse vs sequential.
//!
//! Inputs are synthetic but corpus-shaped: paragraph runs of mixed
//! Japanese prose with sprinkled aozora annotations. Sizes
//! 16/64/256 KB and 1 MB cover both the below-threshold path and the
//! above-threshold path (the threshold is 64 KB).
//!
//! Run via `cargo bench -p aozora-parser --bench parallel_parse`.

use aozora_parser::parallel::parse_sequential;
use aozora_parser::parse;
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

const SIZES: &[usize] = &[
    16 * 1024,        // 16 KB — below threshold (no-op)
    64 * 1024,        // 64 KB — below threshold (no-op)
    256 * 1024,       // 256 KB — at lower edge
    512 * 1024,       // 512 KB — at threshold boundary
    1024 * 1024,      // 1 MB
    3 * 1024 * 1024,  // 3 MB — pathological large doc
];

fn build_corpus_shaped(size_target: usize) -> String {
    // Mix plain prose, ruby annotations, and a paired container so
    // the segmenter has both splittable and unsplittable regions.
    let cell = "春は曙やうやう白くなり行く山際少し明りて紫だちたる雲の細くたなびきたる｜青梅《おうめ》へ走る\n\
                ［＃ここから割書］\n\
                inside the container body — must stay in one segment\n\n\
                more body content\n\n\
                ［＃ここで割書終わり］\n\n\
                another paragraph with no annotations at all\n\n";
    let mut out = String::with_capacity(size_target + cell.len());
    while out.len() < size_target {
        out.push_str(cell);
    }
    out
}

fn bench_parse_size(c: &mut Criterion, label: &str, size: usize) {
    let input = build_corpus_shaped(size);
    let actual = input.len();

    let mut group = c.benchmark_group(format!("parallel_parse/{label}"));
    group.throughput(criterion::Throughput::Bytes(actual as u64));

    group.bench_function("sequential", |b| {
        b.iter(|| {
            drop(black_box(parse_sequential(black_box(&input))));
        });
    });
    group.bench_function("parallel_dispatch", |b| {
        b.iter(|| {
            drop(black_box(parse(black_box(&input))));
        });
    });
    group.finish();
}

fn bench_all(c: &mut Criterion) {
    for &size in SIZES {
        let label = if size >= 1024 * 1024 {
            format!("{}MB", size / (1024 * 1024))
        } else {
            format!("{}KB", size / 1024)
        };
        bench_parse_size(c, &label, size);
    }
}

criterion_group!(benches, bench_all);
criterion_main!(benches);
