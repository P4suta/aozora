//! Editor-burst microbenchmark.
//!
//! Reproduces the keystroke-burst pattern from the user trace
//! (10:05 PM, samples/bouten.afm 6.3 MB, 108k lines, ~24k notation
//! tokens). Splits into focused micro-benches so we can attribute
//! the wall time to specific paths:
//!
//! - `apply_changes/insert_one_char` — single edit cost on a 6 MB
//!   doc. Under the current architecture this also rebuilds
//!   `LineIndex` and re-walks the tree-sitter tree to refresh
//!   `gaiji_spans`. **Hypothesis: dominated by gaiji-span tree walk.**
//! - `apply_changes/burst_100`            — 100 sequential edits.
//!   Linear in count if no per-edit allocation cliff.
//! - `gaiji_span_extract`      — just the tree walk in
//!   `extract_gaiji_spans`. Bounds the cost we'd pay if we
//!   recomputed eagerly.
//! - `line_index_build`        — `LineIndex::new` over 6 MB. Should
//!   be a few ms.
//!
//! Run with `cargo bench -p aozora-lsp --bench burst`.

use std::env;
use std::fs;
use std::hint::black_box;
use std::path::Path;
use std::sync::Arc;

use aozora_lsp::internals::{
    ByteEdit, LineIndex, OpenDocument, TreeSitterDoc, apply_edits, extract_gaiji_spans_from_tree,
    input_edit,
};
use criterion::measurement::WallTime;
use criterion::{BatchSize, BenchmarkGroup, Criterion, criterion_group, criterion_main};

fn load_fixture(name: &str) -> String {
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    // crates/aozora-lsp → ../.. = workspace root
    let path = Path::new(&manifest)
        .join("../..")
        .join("samples")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|_| {
        // The real fixtures (6 MiB bouten/gaiji corpora) are not in git.
        // Fall back to a shape-matched synthetic doc so the bench runs
        // anywhere; drop a real sample into `samples/` for a
        // representative measurement.
        eprintln!(
            "fixture {name} not at {}; using a ~6 MiB synthetic doc instead",
            path.display()
        );
        synthetic_fixture()
    })
}

/// ~6 MiB of blank-line-separated plain prose so the benches are runnable
/// without an external fixture (many paragraphs ⇒ the snapshot-rebuild
/// and segmentation paths are still exercised).
fn synthetic_fixture() -> String {
    const PARA: &str = "これは段落の本文です。それなりの長さの文章がここに続いていきます。\n\n";
    const TARGET: usize = 6 * 1024 * 1024;
    let mut s = String::with_capacity(TARGET + PARA.len());
    while s.len() < TARGET {
        s.push_str(PARA);
    }
    s
}

fn bench_apply_changes(c: &mut Criterion) {
    let text = load_fixture("bouten.afm");
    let mut g = c.benchmark_group("apply_changes");
    g.sample_size(20);
    // NOTE: outside a tokio runtime, `OpenDocument::apply_changes` falls
    // back to a synchronous snapshot rebuild — so this measurement
    // captures the full end-to-end cost (buffer mutate + TS apply_edit
    // + snapshot rebuild). In production the rebuild runs on the
    // tokio blocking pool and `apply_changes` returns in microseconds;
    // we measure the worst-case wall here on purpose.
    g.bench_function("insert_one_char_bouten_6mb", |b| {
        b.iter_batched(
            || OpenDocument::new(text.clone()),
            |state| {
                _ = state.apply_changes(&[ByteEdit::new(0..0, " ".to_owned())]);
            },
            BatchSize::PerIteration,
        );
    });
    g.bench_function("burst_100_inserts_bouten_6mb", |b| {
        b.iter_batched(
            || OpenDocument::new(text.clone()),
            |state| {
                for _ in 0..100 {
                    _ = state.apply_changes(&[ByteEdit::new(0..0, " ".to_owned())]);
                }
            },
            BatchSize::PerIteration,
        );
    });

    // Mid-document edit — exercises the **incremental** snapshot
    // rebuild path. Insert one space at a UTF-8 boundary near the
    // doc midpoint; tree-sitter's `changed_ranges` should localise
    // the work to a small region around the cursor, dropping the
    // gaiji-span re-walk from 67 ms (cold) down to a few hundred μs
    // (carry-forward + sub-walk only).
    g.bench_function("insert_one_char_mid_doc_bouten_6mb", |b| {
        let mid_offset = nearest_char_boundary(&text, text.len() / 2);
        b.iter_batched(
            || OpenDocument::new(text.clone()),
            move |state| {
                _ = state.apply_changes(&[ByteEdit::new(mid_offset..mid_offset, " ".to_owned())]);
            },
            BatchSize::PerIteration,
        );
    });
    g.finish();
}

/// Find the nearest UTF-8 char boundary at or before `target` so we
/// can construct a valid mid-document edit range without triggering
/// `apply_edits`'s `NonCharBoundary` rejection.
fn nearest_char_boundary(text: &str, target: usize) -> usize {
    let mut idx = target.min(text.len());
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// Top-level dispatcher. Each individual bench is its own helper below,
/// keeping `bench_subcomponents` short and each bench discoverable as a
/// named function in stack traces / `cargo bench --bench burst -- <name>`.
fn bench_subcomponents(c: &mut Criterion) {
    let text = load_fixture("bouten.afm");
    let mut g = c.benchmark_group("subcomponents");
    g.sample_size(20);

    bench_line_index_build(&mut g, &text);
    bench_gaiji_span_extract(&mut g, &text);
    bench_ts_parse_full(&mut g, &text);
    bench_apply_edits_insert(&mut g, &text);
    bench_ts_apply_edit_offset_0(&mut g, &text);
    bench_ts_apply_edit_mid_doc(&mut g, &text);
    bench_ts_parse_60kb_slice(&mut g, &text);
    bench_ts_parse_600kb_slice(&mut g, &text);

    g.finish();
}

fn bench_line_index_build(g: &mut BenchmarkGroup<'_, WallTime>, text: &str) {
    g.bench_function("line_index_build_bouten_6mb", |b| {
        b.iter(|| {
            let idx = LineIndex::new(text);
            black_box(idx);
        });
    });
}

fn bench_gaiji_span_extract(g: &mut BenchmarkGroup<'_, WallTime>, text: &str) {
    g.bench_function("gaiji_span_extract_bouten_6mb", |b| {
        b.iter_batched(
            || {
                let doc = TreeSitterDoc::new();
                doc.parse_full(text);
                doc
            },
            |doc| {
                let spans = doc
                    .with_tree(|tree| extract_gaiji_spans_from_tree(tree, text))
                    .unwrap_or_else(|| Arc::from(Vec::new()));
                black_box(spans);
            },
            BatchSize::PerIteration,
        );
    });
}

fn bench_ts_parse_full(g: &mut BenchmarkGroup<'_, WallTime>, text: &str) {
    g.bench_function("ts_parse_full_bouten_6mb", |b| {
        b.iter_batched(
            TreeSitterDoc::new,
            |doc| {
                doc.parse_full(text);
                black_box(doc);
            },
            BatchSize::PerIteration,
        );
    });
}

/// Isolate the pure string splice — `apply_changes` flow does this
/// first, and we suspect it's the dominant cost on a 6 MB buffer
/// because `apply_edits` allocates a fresh `String::with_capacity`
/// and `push_str`s the entire prefix + tail every edit.
fn bench_apply_edits_insert(g: &mut BenchmarkGroup<'_, WallTime>, text: &str) {
    g.bench_function("apply_edits_insert_one_char_bouten_6mb", |b| {
        let edit = vec![ByteEdit::new(0..0, " ".to_owned())];
        b.iter(|| {
            let new_text = apply_edits(text, &edit).expect("valid edit");
            black_box(new_text);
        });
    });
}

/// Isolate the tree-sitter incremental edit at OFFSET 0 — the
/// worst case for incremental reuse because every byte after the
/// edit shifts. tree-sitter must invalidate (almost) every node.
fn bench_ts_apply_edit_offset_0(g: &mut BenchmarkGroup<'_, WallTime>, text: &str) {
    g.bench_function("ts_apply_edit_offset_0_bouten_6mb", |b| {
        b.iter_batched(
            || {
                let doc = TreeSitterDoc::new();
                doc.parse_full(text);
                doc
            },
            |doc| {
                let edit = input_edit(0, 0, 1);
                let mut new_text = String::with_capacity(text.len() + 1);
                new_text.push(' ');
                new_text.push_str(text);
                doc.apply_edit(&new_text, edit);
                black_box(doc);
            },
            BatchSize::PerIteration,
        );
    });
}

/// Same benchmark but the edit is in the middle of the document.
/// Tree-sitter's incremental reparse should reuse most subtrees, so the
/// cost drops well below the offset-0 worst case — the ratio is what
/// this measures.
fn bench_ts_apply_edit_mid_doc(g: &mut BenchmarkGroup<'_, WallTime>, text: &str) {
    g.bench_function("ts_apply_edit_mid_doc_bouten_6mb", |b| {
        let mid = nearest_char_boundary(text, text.len() / 2);
        b.iter_batched(
            || {
                let doc = TreeSitterDoc::new();
                doc.parse_full(text);
                doc
            },
            |doc| {
                let edit = input_edit(mid, mid, mid + 1);
                let mut new_text = String::with_capacity(text.len() + 1);
                new_text.push_str(&text[..mid]);
                new_text.push(' ');
                new_text.push_str(&text[mid..]);
                doc.apply_edit(&new_text, edit);
                black_box(doc);
            },
            BatchSize::PerIteration,
        );
    });
}

/// Cold parse on a small sub-slice of the doc — this is the
/// measurement that motivates per-paragraph segmentation. If a
/// 60 KB paragraph parses in ~3 ms, then a per-paragraph design
/// turns the per-edit TS cost from 220 ms into 3 ms.
fn bench_ts_parse_60kb_slice(g: &mut BenchmarkGroup<'_, WallTime>, text: &str) {
    g.bench_function("ts_parse_full_60kb_slice_bouten", |b| {
        let slice_end = nearest_char_boundary(text, 60 * 1024);
        let small = &text[..slice_end];
        b.iter_batched(
            TreeSitterDoc::new,
            |doc| {
                doc.parse_full(small);
                black_box(doc);
            },
            BatchSize::PerIteration,
        );
    });
}

fn bench_ts_parse_600kb_slice(g: &mut BenchmarkGroup<'_, WallTime>, text: &str) {
    g.bench_function("ts_parse_full_600kb_slice_bouten", |b| {
        let slice_end = nearest_char_boundary(text, 600 * 1024);
        let small = &text[..slice_end];
        b.iter_batched(
            TreeSitterDoc::new,
            |doc| {
                doc.parse_full(small);
                black_box(doc);
            },
            BatchSize::PerIteration,
        );
    });
}

/// Quantify the wait-free read property of the `ArcSwap`-backed
/// snapshot. Two measurements:
///
/// - `snapshot_load_solo` — `state.snapshot()` against a quiescent
///   `OpenDocument`. Should be sub-microsecond (single atomic load + Arc
///   bump).
/// - `snapshot_load_under_write_pressure` — same call while a
///   background thread hammers `apply_changes` on the same state.
///   The architectural claim is "reads never wait on writers", so
///   the per-call latency must remain sub-microsecond — same order
///   of magnitude as the solo case.
///
/// If the solo and under-pressure numbers diverge, the wait-free
/// invariant is broken (the snapshot pointer is being held live
/// somewhere that contends with writers). The bench is a regression
/// gate for the refactor.
fn bench_concurrent_reads(c: &mut Criterion) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;

    let text = load_fixture("bouten.afm");
    let mut g = c.benchmark_group("concurrent_reads");
    g.sample_size(20);

    g.bench_function("snapshot_load_solo_bouten_6mb", |b| {
        let state = OpenDocument::new(text.clone());
        b.iter(|| {
            let snap = state.snapshot();
            black_box(snap);
        });
    });

    g.bench_function("snapshot_load_under_write_pressure_bouten_6mb", |b| {
        let state = OpenDocument::new(text.clone());
        let stop = Arc::new(AtomicBool::new(false));
        let writer_state = Arc::clone(&state);
        let writer_stop = Arc::clone(&stop);
        // Spawn a writer thread that loops `apply_changes`. The
        // synchronous fall-back inside OpenDocument::apply_changes means
        // each write spends ~270 ms holding the buffer mutex; reads
        // hitting the snapshot must be unaffected.
        let writer = thread::spawn(move || {
            let mut i = 0usize;
            while !writer_stop.load(Ordering::Relaxed) {
                _ = writer_state.apply_changes(&[ByteEdit::new(i..i, " ".to_owned())]);
                i += 2; // Skip ahead each round to avoid edits stacking on each other
            }
        });
        b.iter(|| {
            let snap = state.snapshot();
            black_box(snap);
        });
        stop.store(true, Ordering::Relaxed);
        writer.join().expect("writer thread joined");
    });

    g.finish();
}

criterion_group!(
    benches,
    bench_subcomponents,
    bench_apply_changes,
    bench_concurrent_reads
);
criterion_main!(benches);
