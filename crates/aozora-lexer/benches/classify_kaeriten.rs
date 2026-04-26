//! Criterion bench for kaeriten classification through the public
//! borrowed-pipeline entry point.
//!
//! `classify_kaeriten` itself is crate-private, so the bench drives
//! it via [`aozora_lex::lex_into_arena`] on synthetic inputs:
//!
//! - **`kaeriten_dense`** — a buffer dominated by `［＃<mark>］`
//!   annotations whose body is one of the 18 spec marks. Every
//!   annotation hits the `KAERITEN_MARKS.contains` perfect-hash
//!   probe AND the `body.len()` length prefilter.
//! - **`annotations_no_kaeriten`** — same shape, but bodies are
//!   non-kaeriten strings (e.g. random hiragana). Exercises the
//!   negative path where the length prefilter or hash miss returns
//!   `None` quickly.
//!
//! Run via `cargo bench -p aozora-lexer`.

use aozora_lex::lex_into_arena;
use aozora_syntax::borrowed::Arena;
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

const TARGET_BYTES: usize = 32 * 1024;

fn build_kaeriten_dense() -> String {
    // Cycle through every spec mark so the bench exercises the full
    // hash table (canonical 12 + compound 6 = 18).
    let marks = [
        "一", "丁", "三", "上", "下", "中", "丙", "乙", "二", "四", "甲", "レ", "一レ", "上レ",
        "下レ", "中レ", "二レ", "三レ",
    ];
    let mut out = String::with_capacity(TARGET_BYTES);
    let mut i = 0usize;
    while out.len() < TARGET_BYTES {
        let mark = marks[i % marks.len()];
        out.push_str("漢［＃");
        out.push_str(mark);
        out.push_str("］\n");
        i += 1;
    }
    out
}

fn build_annotations_no_kaeriten() -> String {
    // Same density but with annotation bodies that don't match any
    // kaeriten mark. The length prefilter (3 or 6 bytes) will reject
    // most before the hash probe; long bodies bypass the hash entirely.
    let bodies = ["大見出し", "小見出し", "ふりがな", "傍点", "白ゴマ"];
    let mut out = String::with_capacity(TARGET_BYTES);
    let mut i = 0usize;
    while out.len() < TARGET_BYTES {
        let body = bodies[i % bodies.len()];
        out.push_str("漢［＃");
        out.push_str(body);
        out.push_str("］\n");
        i += 1;
    }
    out
}

fn bench_kaeriten(c: &mut Criterion) {
    let mut group = c.benchmark_group("classify_kaeriten");
    let dense = build_kaeriten_dense();
    let absent = build_annotations_no_kaeriten();

    group.bench_function("kaeriten_dense_32k", |b| {
        b.iter(|| {
            let arena = Arena::new();
            black_box(lex_into_arena(black_box(&dense), &arena));
        });
    });
    group.bench_function("annotations_no_kaeriten_32k", |b| {
        b.iter(|| {
            let arena = Arena::new();
            black_box(lex_into_arena(black_box(&absent), &arena));
        });
    });
    group.finish();
}

criterion_group!(benches, bench_kaeriten);
criterion_main!(benches);
