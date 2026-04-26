//! Phase 0 sub-pass downstream impact on phase 1 throughput.
//!
//! For each corpus document this probe detects which sanitize sub-passes
//! actually fire — CRLF rewrite, decorative-rule isolation, accent
//! decompose — and groups documents by the resulting bit-bucket. For
//! each bucket it then re-times phase 1 (`tokenize`) on the post-sanitize
//! text and reports the median ns/byte. The point is to answer
//! "does triggering `rewrite_accent_spans` make the downstream
//! tokenizer measurably slower?" — separating intrinsic
//! document-shape variation from sub-pass-induced text rewriting.
//!
//! The doc-hidden phase 0 helpers (`normalize_line_endings`,
//! `has_long_rule_line`, `isolate_decorative_rules`,
//! `rewrite_accent_spans`) are re-exported by `aozora_lexer`; this probe
//! drives them directly so the gating decisions match the production
//! `sanitize` function exactly.
//!
//! ```text
//! AOZORA_CORPUS_ROOT=… cargo run --release --example phase0_impact -p aozora-bench
//! ```

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::too_many_lines,
    clippy::disallowed_methods,
    reason = "profiling-example tool, not library code"
)]

use std::env;
use std::hint;
use std::process;
use std::time::Instant;

use memchr::memmem;

use aozora_corpus::CorpusItem;
use aozora_encoding::decode_sjis;
use aozora_lexer::{
    has_long_rule_line, isolate_decorative_rules, normalize_line_endings, rewrite_accent_spans,
    tokenize,
};

const TORTOISE_OPEN: &str = "〔";
const BUCKETS: [&str; 8] = [
    "none",      // 0b000
    "crlf",      // 0b001
    "rule",      // 0b010
    "crlf+rule", // 0b011
    "accent",    // 0b100
    "crlf+accent",
    "rule+accent",
    "crlf+rule+accent", // 0b111
];

#[derive(Debug, Default, Clone)]
struct Bucket {
    docs: usize,
    bytes_total: u64,
    tokenize_ns_total: u128,
    /// Per-doc ns/byte for median computation.
    ns_per_byte: Vec<f64>,
}

fn main() {
    let Some(corpus) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set or not a directory; nothing to profile.");
        process::exit(2);
    };
    let limit: Option<usize> = env::var("AOZORA_PROFILE_LIMIT")
        .ok()
        .and_then(|s| s.trim().parse().ok());

    eprintln!("phase0_impact: starting (limit = {limit:?})");

    let items: Vec<CorpusItem> = corpus
        .iter()
        .take(limit.unwrap_or(usize::MAX))
        .filter_map(Result::ok)
        .collect();
    eprintln!("phase0_impact: loaded {} items, measuring…", items.len());

    let mut buckets: [Bucket; 8] = Default::default();
    let mut decode_errors: u64 = 0;
    let wall_start = Instant::now();

    for (i, item) in items.iter().enumerate() {
        let Ok(text) = decode_sjis(&item.bytes) else {
            decode_errors += 1;
            continue;
        };

        // Replicate sanitize's gating decisions, then materialise the
        // post-sanitize text the same way `sanitize` does.
        let after_bom = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
        let has_cr = after_bom.contains('\r');
        let line_norm: String = if has_cr {
            normalize_line_endings(after_bom)
        } else {
            String::new()
        };
        let line_norm_ref: &str = if has_cr { &line_norm } else { after_bom };

        let has_rule = has_long_rule_line(line_norm_ref);
        let rule_iso: String = if has_rule {
            isolate_decorative_rules(line_norm_ref)
        } else {
            String::new()
        };
        let rule_iso_ref: &str = if has_rule { &rule_iso } else { line_norm_ref };

        let has_accent = memmem::find(rule_iso_ref.as_bytes(), TORTOISE_OPEN.as_bytes()).is_some();
        let final_text: String = if has_accent {
            rewrite_accent_spans(rule_iso_ref)
        } else {
            String::new()
        };
        let final_ref: &str = if has_accent {
            &final_text
        } else {
            rule_iso_ref
        };

        // Time tokenize on the post-sanitize text. `tokenize` returns
        // an iterator; `.count()` drains it without allocating a Vec.
        let bytes = final_ref.len() as u64;
        let t = Instant::now();
        let token_count = tokenize(final_ref).count();
        let elapsed = t.elapsed().as_nanos() as u64;
        // Sanity-touch the count so the optimizer can't elide it.
        hint::black_box(token_count);

        let mask: usize =
            (usize::from(has_cr)) | (usize::from(has_rule) << 1) | (usize::from(has_accent) << 2);
        let b = &mut buckets[mask];
        b.docs += 1;
        b.bytes_total += bytes;
        b.tokenize_ns_total += u128::from(elapsed);
        if bytes > 0 {
            b.ns_per_byte.push(elapsed as f64 / bytes as f64);
        }

        if (i + 1).is_multiple_of(2_000) {
            eprintln!(
                "  …processed {} docs ({:.1}s elapsed)",
                i + 1,
                wall_start.elapsed().as_secs_f64()
            );
        }
    }
    eprintln!(
        "phase0_impact: done in {:.2}s, {decode_errors} decode errors",
        wall_start.elapsed().as_secs_f64()
    );

    print_report(&buckets, decode_errors);
}

fn print_report(buckets: &[Bucket; 8], decode_errors: u64) {
    println!("=== phase0_impact (sub-pass → tokenize ns/byte) ===");
    println!();
    println!("decode errors: {decode_errors}");
    println!();
    println!(
        "{:<20} {:>8} {:>14} {:>14} {:>14} {:>14}",
        "sub-passes", "docs", "bytes total", "tok ms total", "agg ns/byte", "p50 ns/byte"
    );
    println!("{}", "-".repeat(89));
    for (mask, b) in buckets.iter().enumerate() {
        let label = BUCKETS[mask];
        if b.docs == 0 {
            println!(
                "{label:<20} {:>8} {:>14} {:>14} {:>14} {:>14}",
                0, "-", "-", "-", "-"
            );
            continue;
        }
        let agg = if b.bytes_total == 0 {
            0.0
        } else {
            b.tokenize_ns_total as f64 / b.bytes_total as f64
        };
        let p50 = {
            let mut v = b.ns_per_byte.clone();
            v.sort_by(|a, c| a.partial_cmp(c).expect("no NaN"));
            v.get(v.len() / 2).copied().unwrap_or(0.0)
        };
        println!(
            "{label:<20} {:>8} {:>14} {:>14.2} {:>14.2} {:>14.2}",
            b.docs,
            b.bytes_total,
            b.tokenize_ns_total as f64 / 1_000_000.0,
            agg,
            p50,
        );
    }
    println!();
    println!("Compare 'none' to single-flag buckets to see the per-pass downstream cost.");
}
