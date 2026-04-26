//! Single-document criterion benchmark using `罪と罰` (米川正夫訳).
//!
//! The corpus root is read from the `AOZORA_CORPUS_ROOT` environment
//! variable; the bench skips with a notice if the file is missing,
//! so a fresh checkout without a corpus continues to compile + run
//! the rest of the bench suite.

#![allow(
    clippy::missing_panics_doc,
    reason = "bench code, not library"
)]

use std::env;
use std::fs;
use std::hint::black_box;
use std::path::PathBuf;

use aozora::Document;
use aozora_encoding::decode_sjis;
use criterion::{Criterion, Throughput, criterion_group, criterion_main};

const RELATIVE_PATH: &str = "000363/files/56656_ruby_74439/56656_ruby_74439.txt";

fn locate_crime_and_punishment() -> Option<PathBuf> {
    let root = env::var_os("AOZORA_CORPUS_ROOT")?;
    let candidate = PathBuf::from(root).join(RELATIVE_PATH);
    candidate.is_file().then_some(candidate)
}

fn bench_crime_and_punishment(c: &mut Criterion) {
    let Some(path) = locate_crime_and_punishment() else {
        eprintln!(
            "AOZORA_CORPUS_ROOT not set or 罪と罰 not present at {RELATIVE_PATH}; skipping bench"
        );
        return;
    };
    let bytes = fs::read(&path).expect("read 罪と罰");
    let utf8 = decode_sjis(&bytes).expect("decode SJIS");

    let mut g = c.benchmark_group("crime_and_punishment");
    g.throughput(Throughput::Bytes(utf8.len() as u64));

    g.bench_function("parse", |b| {
        b.iter(|| {
            let doc = Document::new(black_box(utf8.as_str()));
            let tree = doc.parse();
            black_box(tree);
        });
    });

    g.bench_function("parse_then_html", |b| {
        b.iter(|| {
            let doc = Document::new(black_box(utf8.as_str()));
            let tree = doc.parse();
            let html = tree.to_html();
            black_box(html);
        });
    });

    g.bench_function("parse_then_serialize", |b| {
        b.iter(|| {
            let doc = Document::new(black_box(utf8.as_str()));
            let tree = doc.parse();
            let out = tree.serialize();
            black_box(out);
        });
    });

    g.finish();
}

criterion_group!(crime_benches, bench_crime_and_punishment);
criterion_main!(crime_benches);
