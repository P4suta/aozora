//! Empirical byte-density probe — measures the distribution of
//! trigger-leading vs trigger-middle bytes across the corpus.
//!
//! On Japanese text the leading-byte set `{0xE2, 0xE3, 0xEF}` is
//! dominated by ~33 % 0xE3 density (every hiragana / katakana / many
//! CJK starts with it). The middle-byte set `{0x80, 0xBC, 0xBD}` is
//! several × sparser and the more useful candidate filter — which is
//! why `aozora-scan` ships Teddy / structural-bitmap rather than a
//! leading-byte memchr3 scan.
//!
//! Output is intentionally `println!`-formatted with raw counts +
//! percentages.

#![allow(
    clippy::cast_precision_loss,
    clippy::missing_panics_doc,
    reason = "probe code: integer-to-float for percentage formatting only"
)]

use std::env;
use std::process::ExitCode;

use aozora_corpus::CorpusItem;
use aozora_encoding::decode_sjis;
use aozora_scan::{NaiveScanner, TriggerScanner};

const DEFAULT_LIMIT: usize = 2000;

fn main() -> ExitCode {
    let Some(corpus) = aozora_corpus::from_env() else {
        eprintln!("AOZORA_CORPUS_ROOT not set");
        return ExitCode::from(2);
    };

    let limit: usize = env::var("AOZORA_PROFILE_LIMIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_LIMIT);

    let items: Vec<CorpusItem> = corpus.iter().take(limit).filter_map(Result::ok).collect();

    let mut totals = ByteCounts::default();
    let mut docs = 0u64;

    for item in &items {
        let Ok(text) = decode_sjis(&item.bytes) else {
            continue;
        };
        totals.observe(&text);
        docs += 1;
    }

    print_report(docs, &totals);
    ExitCode::SUCCESS
}

#[derive(Default)]
struct ByteCounts {
    total_bytes: u64,
    e2: u64,
    e3: u64,
    ef: u64,
    b80: u64,
    bbc: u64,
    bbd: u64,
    triggers: u64,
}

impl ByteCounts {
    fn observe(&mut self, text: &str) {
        let bytes = text.as_bytes();
        self.total_bytes += bytes.len() as u64;
        for &b in bytes {
            match b {
                0xE2 => self.e2 += 1,
                0xE3 => self.e3 += 1,
                0xEF => self.ef += 1,
                0x80 => self.b80 += 1,
                0xBC => self.bbc += 1,
                0xBD => self.bbd += 1,
                _ => {}
            }
        }
        // NaiveScanner is the ground truth — independent of the
        // SIMD scanner whose design this probe informs.
        self.triggers += NaiveScanner.scan_offsets(text).len() as u64;
    }
}

fn print_report(docs: u64, c: &ByteCounts) {
    let total = c.total_bytes as f64;
    let pct = |n: u64| n as f64 * 100.0 / total;

    println!("docs analyzed: {docs}");
    println!("total bytes:   {}", c.total_bytes);
    println!();
    println!("LEADING-byte candidates ({{0xE2, 0xE3, 0xEF}} — v1 scan target):");
    println!("  0xE2: {:>12} ({:.3} % of bytes)", c.e2, pct(c.e2));
    println!("  0xE3: {:>12} ({:.3} % of bytes)", c.e3, pct(c.e3));
    println!("  0xEF: {:>12} ({:.3} % of bytes)", c.ef, pct(c.ef));
    let lead_total = c.e2 + c.e3 + c.ef;
    println!(
        "  TOTAL:{:>12} ({:.3} % of bytes)",
        lead_total,
        pct(lead_total)
    );
    println!();
    println!("MIDDLE-byte candidates ({{0x80, 0xBC, 0xBD}} — v2 structural-bitmap target):");
    println!("  0x80: {:>12} ({:.3} % of bytes)", c.b80, pct(c.b80));
    println!("  0xBC: {:>12} ({:.3} % of bytes)", c.bbc, pct(c.bbc));
    println!("  0xBD: {:>12} ({:.3} % of bytes)", c.bbd, pct(c.bbd));
    let mid_total = c.b80 + c.bbc + c.bbd;
    println!(
        "  TOTAL:{:>12} ({:.3} % of bytes)",
        mid_total,
        pct(mid_total)
    );
    println!();
    println!(
        "True triggers: {} ({:.4} % of bytes)",
        c.triggers,
        pct(c.triggers)
    );
    println!();
    println!("Candidate-to-trigger ratios (lower = better):");
    let trigs = c.triggers.max(1) as f64;
    println!(
        "  Leading-byte: {:>5.1}× false positives per real trigger",
        lead_total as f64 / trigs
    );
    println!(
        "  Middle-byte:  {:>5.1}× false positives per real trigger",
        mid_total as f64 / trigs
    );
    println!();
    println!("Speedup ratio (theoretical, candidate-finding stage):");
    println!(
        "  Middle-byte / Leading-byte: {:.2}×",
        lead_total as f64 / mid_total.max(1) as f64
    );
}
