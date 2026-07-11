//! Instruction-count perf gate (G6) — deterministic Callgrind micro-benchmarks.
//!
//! Wall-clock timing on shared CI runners is too noisy to gate on (the
//! `just throughput` recipe carries the same warning). This bench instead
//! runs under Valgrind's Callgrind, which counts CPU **instructions** — a
//! figure that is deterministic across runs and machines. A regression of
//! more than 10% in `Ir` (instructions read) on any case fails the run.
//!
//! Corpus-free by construction: it embeds a handful of real vendored
//! 青空文庫 works (from the conformance fixture corpus, the single source
//! of truth) plus a synthetic annotation-dense buffer, so it needs no
//! `AOZORA_CORPUS_ROOT`. Driven by `just perf-gate` (first run records a
//! baseline; later runs compare against it).
//!
//! Requires `valgrind` and a matching `iai-callgrind-runner` on PATH; both
//! are baked into the dev image (see the Dockerfile).

#![allow(
    missing_docs,
    unused_qualifications,
    clippy::disallowed_methods,
    clippy::missing_panics_doc,
    reason = "iai-callgrind's library_benchmark / library_benchmark_group / main \
              macros expand to undocumented, fully-qualified items this crate does \
              not own (the `main!` harness exits via std::process::exit); bench fns \
              are not a public panics-documented surface"
)]

use std::hint::black_box;

use aozora::Document;
use aozora_bench::build_pathological_aozora;
use iai_callgrind::{
    Callgrind, EventKind, LibraryBenchmarkConfig, library_benchmark, library_benchmark_group, main,
};

// Real vendored 青空文庫 works, embedded at build time from the conformance
// fixture corpus so this gate stays corpus-free and portable. include_str!
// resolves relative to THIS file, reaching into the sibling crate's
// fixtures — the single vendored copy, never duplicated here.
const TOSA: &str = include_str!("../../aozora-conformance/fixtures/works/terada-tosa/source.txt");
const MATOI: &str =
    include_str!("../../aozora-conformance/fixtures/works/orikuchi-matoi/source.txt");
const PETER: &str = include_str!("../../aozora-conformance/fixtures/works/potter-peter/source.txt");

/// Approximate size of the synthetic classify-density stressor. Kept modest
/// because callgrind simulates every instruction (~50× slower than native).
const PATHOLOGICAL_BYTES: usize = 64 * 1024;

// Parse only — the hot lex → classify → arena pipeline. Touching the
// diagnostics count forces the parse (the optimizer cannot elide it).
// (Plain `//` comments, not `///`: the `#[library_benchmark]` macro rejects
// any attribute — including a doc attribute — that is not `#[bench]`.)
#[library_benchmark]
#[bench::tosa(TOSA)]
#[bench::matoi(MATOI)]
#[bench::peter(PETER)]
fn parse(src: &str) -> usize {
    let doc = Document::new(black_box(src));
    let tree = doc.parse();
    black_box(tree.diagnostics().len())
}

// Parse then render to HTML — the full end-to-end path a CLI `render`
// invocation walks.
#[library_benchmark]
#[bench::tosa(TOSA)]
#[bench::matoi(MATOI)]
fn parse_then_html(src: &str) -> String {
    let doc = Document::new(black_box(src));
    black_box(doc.parse().to_html())
}

// Classify-density outlier: the annotation-dense synthetic buffer. The
// `setup` builds the string OUTSIDE the measured region, so only the parse
// is counted.
#[library_benchmark]
#[bench::dense(args = (PATHOLOGICAL_BYTES), setup = build_pathological_aozora)]
fn parse_pathological(src: String) -> usize {
    let doc = Document::new(black_box(src));
    let tree = doc.parse();
    black_box(tree.diagnostics().len())
}

library_benchmark_group!(
    name = perf_gate;
    benchmarks = parse, parse_then_html, parse_pathological
);

// The soft limit turns a >10% `Ir` change (relative to the saved baseline)
// into a non-zero exit after the whole run — the perf gate's fail signal.
main!(
    config = LibraryBenchmarkConfig::default()
        .tool(Callgrind::default().soft_limits([(EventKind::Ir, 10.0)]));
    library_benchmark_groups = perf_gate
);
