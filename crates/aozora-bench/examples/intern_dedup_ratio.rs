//! Measure the ArenaInterner's deduplication effect on the
//! 17 k-document Aozora corpus.
//!
//! For each document we run `aozora_lex::lex_into_arena` (which uses
//! the interner internally) and accumulate the `InternStats` exposed
//! on the resulting `BorrowedLexOutput`. The aggregate stats then
//! report:
//!
//! - Total interned strings vs unique allocations: dedup ratio.
//! - Cache hit rate (consecutive identical interns).
//! - Average probe length (hash table health).
//!
//! Set `AOZORA_CORPUS_ROOT` to the corpus root before running.
//!
//! ```bash
//! AOZORA_CORPUS_ROOT=/path/to/aozorabunko_text-master/cards \
//!   cargo run --release -p aozora-bench --example intern_dedup_ratio
//! ```

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::disallowed_methods,
    clippy::too_many_lines,
    reason = "profiling-example tool"
)]

use std::env;
use std::time::Instant;

use aozora_corpus::{CorpusSource, FilesystemCorpus};
use aozora_encoding::decode_sjis;
use aozora_lex::lex_into_arena;
use aozora_syntax::borrowed::{Arena, InternStats};

fn main() {
    let root = match env::var("AOZORA_CORPUS_ROOT") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("AOZORA_CORPUS_ROOT not set; aborting");
            std::process::exit(2);
        }
    };
    eprintln!("intern_dedup_ratio: scanning {root}");

    let corpus = FilesystemCorpus::new(root).expect("filesystem root must exist");
    let items: Vec<_> = corpus.iter().filter_map(Result::ok).collect();
    eprintln!("intern_dedup_ratio: discovered {} items", items.len());

    let mut total_alloc_bytes: u64 = 0;
    let mut total_documents: u64 = 0;
    let mut total_decode_errors: u64 = 0;
    let mut agg = InternStats::default();
    let start = Instant::now();

    for item in &items {
        let text = match decode_sjis(&item.bytes) {
            Ok(t) => t,
            Err(_) => {
                total_decode_errors += 1;
                continue;
            }
        };
        let arena = Arena::new();
        let out = lex_into_arena(&text, &arena);
        agg.calls += out.intern_stats.calls;
        agg.cache_hits += out.intern_stats.cache_hits;
        agg.table_hits += out.intern_stats.table_hits;
        agg.allocs += out.intern_stats.allocs;
        agg.long_bypass += out.intern_stats.long_bypass;
        agg.resizes += out.intern_stats.resizes;
        agg.probe_steps += out.intern_stats.probe_steps;
        total_alloc_bytes += arena.allocated_bytes() as u64;
        total_documents += 1;
    }

    let elapsed = start.elapsed();
    eprintln!(
        "intern_dedup_ratio: done in {:.2}s, {total_decode_errors} decode errors",
        elapsed.as_secs_f64()
    );

    let reuses = agg.cache_hits + agg.table_hits;
    let dedup_ratio = if agg.calls == 0 {
        0.0
    } else {
        reuses as f64 / agg.calls as f64
    };
    let avg_probe = {
        let probed = agg.calls.saturating_sub(agg.cache_hits);
        if probed == 0 {
            0.0
        } else {
            agg.probe_steps as f64 / probed as f64
        }
    };

    println!("=== ArenaInterner dedup report (corpus aggregate) ===");
    println!("documents processed       : {total_documents}");
    println!("decode errors             : {total_decode_errors}");
    println!("intern() calls            : {}", agg.calls);
    println!(
        "  cache hits              : {} ({:.1}%)",
        agg.cache_hits,
        pct(agg.cache_hits, agg.calls)
    );
    println!(
        "  table hits              : {} ({:.1}%)",
        agg.table_hits,
        pct(agg.table_hits, agg.calls)
    );
    println!(
        "  fresh allocs            : {} ({:.1}%)",
        agg.allocs,
        pct(agg.allocs, agg.calls)
    );
    println!(
        "  long-bypass allocs      : {} ({:.1}%)",
        agg.long_bypass,
        pct(agg.long_bypass, agg.calls)
    );
    println!("table resizes (total)     : {}", agg.resizes);
    println!("avg probe length          : {avg_probe:.2}");
    println!();
    println!(
        "dedup ratio (reuses/total) : {:.1}%",
        dedup_ratio * 100.0
    );
    println!(
        "  => for every 100 string fields, ~{:.0} skip allocation",
        dedup_ratio * 100.0
    );
    println!();
    println!(
        "arena bytes allocated     : {total_alloc_bytes} bytes ({} MB)",
        total_alloc_bytes / 1_000_000
    );
}

fn pct(num: u64, denom: u64) -> f64 {
    if denom == 0 {
        0.0
    } else {
        (num as f64) / (denom as f64) * 100.0
    }
}
