//! Criterion bench for `parse_incremental` plain-text fast path.
//!
//! Models LSP keystroke traffic: a long plain-text buffer with single
//! 1-byte insertions applied one at a time. The fast path's hot
//! predicate is `is_plain_phase0_clean`, which the optimisation pass
//! reduced from 5 sequential `text.contains()` scans to one
//! `chars().all()` pass with an ASCII u128 bitmap + non-ASCII
//! `matches!`.
//!
//! Run via `cargo bench -p aozora-parser`.

use aozora_parser::{TextEdit, parse, parse_incremental};
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

const PLAIN_BUFFER_BYTES: usize = 64 * 1024;

fn build_plain_buffer() -> String {
    let cell = "the quick brown fox jumps over the lazy dog\n";
    let mut out = String::with_capacity(PLAIN_BUFFER_BYTES + cell.len());
    while out.len() < PLAIN_BUFFER_BYTES {
        out.push_str(cell);
    }
    out
}

fn build_japanese_plain() -> String {
    // No aozora triggers; just hiragana/kanji prose. Exercises the
    // non-ASCII matches! arm of `is_phase0_dirty_char`.
    let cell = "春は曙やうやう白くなり行く山際少し明りて紫だちたる雲の細くたなびきたる\n";
    let mut out = String::with_capacity(PLAIN_BUFFER_BYTES + cell.len());
    while out.len() < PLAIN_BUFFER_BYTES {
        out.push_str(cell);
    }
    out
}

fn bench_fastpath(c: &mut Criterion) {
    let mut group = c.benchmark_group("incremental_fastpath");

    let plain_ascii = build_plain_buffer();
    let plain_jp = build_japanese_plain();
    let prev_ascii = parse(&plain_ascii);
    let prev_jp = parse(&plain_jp);

    // Single-byte insertion at offset 100 (deep inside the buffer)
    // to exercise the realistic LSP keystroke pattern: a plain-text
    // doc receives one tiny edit, fast path fires, parse re-emits
    // the trivial result.
    let edit_ascii = vec![TextEdit::new(100..100, "x".to_owned())];
    let edit_jp = vec![TextEdit::new(99..99, "x".to_owned())];

    group.bench_function("ascii_64k_buffer_one_byte_insert", |b| {
        b.iter(|| {
            let outcome =
                parse_incremental(black_box(&prev_ascii), black_box(&plain_ascii), &edit_ascii)
                    .unwrap();
            black_box(outcome);
        });
    });
    group.bench_function("japanese_64k_buffer_one_byte_insert", |b| {
        b.iter(|| {
            let outcome =
                parse_incremental(black_box(&prev_jp), black_box(&plain_jp), &edit_jp).unwrap();
            black_box(outcome);
        });
    });
    group.finish();
}

criterion_group!(benches, bench_fastpath);
criterion_main!(benches);
