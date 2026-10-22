//! Profiles heap allocations for parsing and rendering synthetic source with `dhat`.

use std::hint::black_box;

use aozora_bench::build_synthetic_aozora;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[expect(
    clippy::expect_used,
    reason = "benchmark inputs must meet parser and numeric preconditions"
)]
fn main() {
    let profiler = dhat::Profiler::new_heap();

    let source = build_synthetic_aozora(2 * 1024 * 1024);
    let doc = aozora::parse(black_box(source.as_str())).expect("source fits parser span limit");
    let tree = doc.snapshot();
    let html = tree.to_html();
    black_box(html);

    drop(profiler);
}
