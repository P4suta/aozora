//! Reports parse and render latency percentiles for synthetic source.

use std::hint::black_box;
use std::time::Instant;

use aozora_bench::build_synthetic_aozora;

const WARMUP: usize = 200;
const ITERS: usize = 4000;

#[expect(
    clippy::expect_used,
    clippy::print_stdout,
    reason = "benchmark commands report measurements and stop on invalid benchmark inputs"
)]
fn main() {
    let source = build_synthetic_aozora(4 * 1024);

    for _ in 0..WARMUP {
        let doc = aozora::parse(black_box(source.as_str())).expect("source fits parser span limit");
        black_box(doc.snapshot().to_html());
    }

    let mut samples_ns: Vec<u128> = Vec::with_capacity(ITERS);
    for _ in 0..ITERS {
        let start = Instant::now();
        let doc = aozora::parse(black_box(source.as_str())).expect("source fits parser span limit");
        let html = doc.snapshot().to_html();
        let elapsed = start.elapsed().as_nanos();
        black_box(html);
        samples_ns.push(elapsed);
    }
    samples_ns.sort_unstable();

    let pct = |p: usize| -> u128 {
        let idx = (samples_ns.len() * p / 100).min(samples_ns.len().saturating_sub(1));
        samples_ns[idx]
    };

    println!(
        "Document parse+to_html small-doc latency ({} bytes, {ITERS} runs)",
        source.len()
    );
    println!("  p50 = {} µs", pct(50) / 1_000);
    println!("  p90 = {} µs", pct(90) / 1_000);
    println!("  p99 = {} µs", pct(99) / 1_000);
    println!("  max = {} µs", samples_ns[samples_ns.len() - 1] / 1_000);
}
