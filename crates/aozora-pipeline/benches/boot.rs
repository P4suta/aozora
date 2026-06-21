//! Criterion bench isolating the parser's *boot* cost — the one-time
//! lazy initialisation that `aozora::prewarm` forces up front.
//!
//! The per-parse benches (`tokenize_compare`, `classify_kaeriten`, and
//! `aozora-bench`'s `crime_and_punishment`) run many in-process
//! iterations, so the first-iteration boot cost is amortised to ~zero
//! and stays invisible. This bench measures the dominant, isolable half
//! directly:
//!
//! - **`aho_corasick_build`** — the classify-stage annotation-classifier DFA
//!   built from the full `BODY_PATTERNS` set, the bulk of boot cost. The
//!   process `OnceLock` behind `body_dispatcher` is unresettable, so the
//!   bench calls the (doc-hidden) builder directly, rebuilding a fresh
//!   automaton each iteration rather than reading the cached getter.
//!
//! The other lazy init `prewarm` forces — the tokenize-stage SIMD backend
//! choice (`aozora_scan`) — is a single `is_x86_feature_detected!` probe
//! whose result `std_detect` caches in a process-global atomic on first
//! use. That cache is unresettable, so the cold one-time cost cannot be
//! isolated in-process (criterion's warmup would prime it); it is a
//! single CPUID, well under a microsecond and dwarfed by the ~150 µs DFA
//! build, so it is not benched separately.
//!
//! Run via `cargo bench -p aozora-pipeline --bench boot`.

use aozora_pipeline::lexer::classify::build_body_dispatcher;
use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

fn bench_boot(c: &mut Criterion) {
    let mut group = c.benchmark_group("boot");
    group.bench_function("aho_corasick_build", |b| {
        b.iter(|| black_box(build_body_dispatcher()));
    });
    group.finish();
}

criterion_group!(benches, bench_boot);
criterion_main!(benches);
