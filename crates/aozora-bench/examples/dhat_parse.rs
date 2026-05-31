//! Heap-allocation profile of a parse + HTML render, via dhat.
//!
//! Reports total allocations + bytes and peak (`At t-gmax`) resident
//! bytes for one 2 MiB synthetic-corpus `Document::new().parse().to_html()`
//! — the memory metric that pairs with the criterion throughput
//! (`synthetic_corpus`) and the `latency_histogram` percentiles. Most of
//! the parser's working set lives in the bumpalo arena, so dhat shows the
//! arena chunk allocations plus any per-parse heap `Vec`s (e.g. the
//! Phase-4 `ArenaNormalizer` builder buffers). Also writes
//! `dhat-heap.json` (viewable at <https://nnethercote.github.io/dh_view/dh_view.html>).
//!
//! ```text
//! just dhat   # docker compose run --rm dev cargo run --release -p aozora-bench --example dhat_parse
//! ```

use std::hint::black_box;

use aozora::Document;
use aozora_bench::build_synthetic_aozora;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    let profiler = dhat::Profiler::new_heap();

    let source = build_synthetic_aozora(2 * 1024 * 1024);
    let doc = Document::new(black_box(source.as_str()));
    let tree = doc.parse();
    let html = tree.to_html();
    black_box(html);

    // Explicit drop so the heap snapshot (summary + dhat-heap.json) is
    // taken here, after one full parse + render.
    drop(profiler);
}
