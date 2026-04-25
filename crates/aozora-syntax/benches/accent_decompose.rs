//! Criterion bench for `aozora_syntax::accent::decompose_fragment`.
//!
//! Three workloads exercise the two hot paths:
//!
//! - **`pure_japanese_no_markers`** — 100KB of Japanese prose with no
//!   ASCII marker bytes at all. Hits the `is_accent_marker` u128
//!   bitmap early-out exclusively. Pins the cost of the membership
//!   prefilter (replaces the prior `ACCENT_MARKERS.contains(&b)`
//!   linear scan).
//! - **`mixed_1pct_accent`** — Japanese prose with 1% of bytes being
//!   accent-bearing Latin words. Forces the longest-match table
//!   lookup (`match_ligature` 4-arm match + `ACCENT_DIGRAPHS` phf
//!   probe). Pins the cost of the per-hit lookup.
//! - **`pure_accent_text`** — 100KB of ASCII-only accent-bearing
//!   words. Worst case for the early-out; every byte feeds the
//!   `ACCENT_DIGRAPHS` probe.
//!
//! Run via `cargo bench -p aozora-syntax`.

use aozora_syntax::accent::decompose_fragment;
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

const ONE_HUNDRED_KB: usize = 100 * 1024;

fn build_pure_japanese() -> String {
    let cell = "春は曙、やうやう白くなり行く山際、少し明りて、紫だちたる雲の細くたなびきたる。";
    let mut out = String::with_capacity(ONE_HUNDRED_KB + cell.len());
    while out.len() < ONE_HUNDRED_KB {
        out.push_str(cell);
    }
    out
}

fn build_mixed_1pct() -> String {
    // ~99% Japanese filler, ~1% accent words.
    let filler = "春は曙、やうやう白くなり行く山際、少し明りて、紫だちたる雲の細くたなびきたる。";
    let accent = "fune`bre ve'rite' ae&on";
    let mut out = String::with_capacity(ONE_HUNDRED_KB + filler.len());
    let mut i = 0usize;
    while out.len() < ONE_HUNDRED_KB {
        out.push_str(filler);
        if i.is_multiple_of(100) {
            out.push_str(accent);
        }
        i += 1;
    }
    out
}

fn build_pure_accent() -> String {
    let cell = "fune`bre ve'rite' ae&on stras&e d/o_g C,a va^ A` E' N~ ";
    let mut out = String::with_capacity(ONE_HUNDRED_KB + cell.len());
    while out.len() < ONE_HUNDRED_KB {
        out.push_str(cell);
    }
    out
}

fn bench_decompose(c: &mut Criterion) {
    let mut group = c.benchmark_group("accent_decompose");
    let pure_japanese = build_pure_japanese();
    let mixed = build_mixed_1pct();
    let pure_accent = build_pure_accent();

    group.bench_function("pure_japanese_no_markers_100k", |b| {
        b.iter(|| {
            drop(black_box(decompose_fragment(black_box(&pure_japanese))));
        });
    });
    group.bench_function("mixed_1pct_accent_100k", |b| {
        b.iter(|| {
            drop(black_box(decompose_fragment(black_box(&mixed))));
        });
    });
    group.bench_function("pure_accent_text_100k", |b| {
        b.iter(|| {
            drop(black_box(decompose_fragment(black_box(&pure_accent))));
        });
    });
    group.finish();
}

criterion_group!(benches, bench_decompose);
criterion_main!(benches);
