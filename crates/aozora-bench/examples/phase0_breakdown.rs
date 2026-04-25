//! Per-sub-pass timing breakdown of phase 0 sanitize.
//!
//! Phase 0 currently accounts for ~18.9% of corpus parse wall-clock.
//! After Plan E (memchr-based PUA collision scan, ~580 MB/s), the
//! residual cost is split across the remaining sub-passes:
//!
//! - BOM strip (`strip_prefix("\u{FEFF}")`)
//! - Line-ending normalisation (CRLF→LF) — gated on `\r` presence
//! - Decorative rule isolation — gated on `has_long_rule_line`
//! - Accent decomposition inside `〔...〕` — gated on TORTOISE_OPEN
//! - PUA sentinel collision scan
//!
//! This bench measures each sub-pass independently across the corpus
//! so we can identify the next residual hotspot.
//!
//! Run via:
//! ```bash
//! AOZORA_CORPUS_ROOT=/path/to/corpus \
//!   cargo run --release --example phase0_breakdown -p aozora-bench
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
    reason = "profiling-example tool"
)]

use std::env;
use std::time::Instant;

use aozora_corpus::{CorpusItem, CorpusSource, FilesystemCorpus};
use aozora_encoding::decode_sjis;
use aozora_lexer::{
    has_long_rule_line, isolate_decorative_rules, normalize_line_endings,
    rewrite_accent_spans, scan_for_sentinel_collisions,
};

const NS_PER_MS: f64 = 1_000_000.0;
const BOM: char = '\u{FEFF}';
const TORTOISE_OPEN_BYTES: &[u8] = "〔".as_bytes();

#[derive(Debug, Clone, Copy, Default)]
struct Sub {
    bytes_in: u64,
    bom_strip_ns: u64,
    crlf_ns: u64,
    crlf_taken: bool,
    rule_gate_ns: u64,
    rule_isolate_ns: u64,
    rule_taken: bool,
    accent_gate_ns: u64,
    accent_rewrite_ns: u64,
    accent_taken: bool,
    pua_scan_ns: u64,
    total_ns: u64,
}

fn main() {
    let root = match env::var("AOZORA_CORPUS_ROOT") {
        Ok(p) => p,
        Err(_) => {
            eprintln!("AOZORA_CORPUS_ROOT not set; aborting");
            std::process::exit(2);
        }
    };
    eprintln!("phase0_breakdown: scanning {root}");

    let corpus = FilesystemCorpus::new(root).expect("filesystem root must exist");
    let items: Vec<CorpusItem> = corpus.iter().filter_map(Result::ok).collect();
    eprintln!("phase0_breakdown: discovered {} items", items.len());

    let mut samples: Vec<Sub> = Vec::with_capacity(items.len());
    let mut decode_errors = 0u64;
    let start = Instant::now();
    for item in &items {
        let text = match decode_sjis(&item.bytes) {
            Ok(t) => t,
            Err(_) => {
                decode_errors += 1;
                continue;
            }
        };
        samples.push(measure_one(&text));
    }
    let wall = start.elapsed();
    eprintln!(
        "phase0_breakdown: done in {:.2}s, {decode_errors} decode errors",
        wall.as_secs_f64()
    );
    print_report(&samples);
}

fn measure_one(text: &str) -> Sub {
    let bytes_in = text.len() as u64;

    // Sub 1: BOM strip — `strip_prefix` is O(1) (just a 3-byte compare),
    // but include it for completeness.
    let t = Instant::now();
    let after_bom = text.strip_prefix(BOM).unwrap_or(text);
    let bom_strip_ns = t.elapsed().as_nanos() as u64;

    // Sub 2: CRLF normalisation gate + transform.
    let t = Instant::now();
    let has_cr = after_bom.contains('\r');
    let line_normalized: String = if has_cr {
        normalize_line_endings(after_bom)
    } else {
        String::new() // sentinel; we won't use it
    };
    let crlf_ns = t.elapsed().as_nanos() as u64;
    let line_normalized_ref: &str = if has_cr { &line_normalized } else { after_bom };

    // Sub 3: rule-line gate + isolation.
    let t = Instant::now();
    let has_rule = has_long_rule_line(line_normalized_ref);
    let rule_gate_ns = t.elapsed().as_nanos() as u64;
    let t = Instant::now();
    let rule_isolated: String = if has_rule {
        isolate_decorative_rules(line_normalized_ref)
    } else {
        String::new()
    };
    let rule_isolate_ns = t.elapsed().as_nanos() as u64;
    let rule_isolated_ref: &str = if has_rule { &rule_isolated } else { line_normalized_ref };

    // Sub 4: accent rewrite gate + transform.
    let t = Instant::now();
    let has_tortoise =
        memchr::memmem::find(rule_isolated_ref.as_bytes(), TORTOISE_OPEN_BYTES).is_some();
    let accent_gate_ns = t.elapsed().as_nanos() as u64;
    let t = Instant::now();
    let final_text: String = if has_tortoise {
        rewrite_accent_spans(rule_isolated_ref)
    } else {
        String::new()
    };
    let accent_rewrite_ns = t.elapsed().as_nanos() as u64;
    let final_text_ref: &str = if has_tortoise { &final_text } else { rule_isolated_ref };

    // Sub 5: PUA sentinel scan.
    let t = Instant::now();
    let _diag = scan_for_sentinel_collisions(final_text_ref);
    let pua_scan_ns = t.elapsed().as_nanos() as u64;

    let total_ns = bom_strip_ns
        + crlf_ns
        + rule_gate_ns
        + rule_isolate_ns
        + accent_gate_ns
        + accent_rewrite_ns
        + pua_scan_ns;

    Sub {
        bytes_in,
        bom_strip_ns,
        crlf_ns,
        crlf_taken: has_cr,
        rule_gate_ns,
        rule_isolate_ns,
        rule_taken: has_rule,
        accent_gate_ns,
        accent_rewrite_ns,
        accent_taken: has_tortoise,
        pua_scan_ns,
        total_ns,
    }
}

fn print_report(samples: &[Sub]) {
    let n = samples.len() as u64;
    let total_ns: u64 = samples.iter().map(|s| s.total_ns).sum();
    let bytes: u64 = samples.iter().map(|s| s.bytes_in).sum();

    let bom_ns: u64 = samples.iter().map(|s| s.bom_strip_ns).sum();
    let crlf_gate_ns: u64 = samples
        .iter()
        .filter(|s| !s.crlf_taken)
        .map(|s| s.crlf_ns)
        .sum();
    let crlf_take_ns: u64 = samples
        .iter()
        .filter(|s| s.crlf_taken)
        .map(|s| s.crlf_ns)
        .sum();
    let rule_gate_ns: u64 = samples.iter().map(|s| s.rule_gate_ns).sum();
    let rule_take_ns: u64 = samples
        .iter()
        .filter(|s| s.rule_taken)
        .map(|s| s.rule_isolate_ns)
        .sum();
    let accent_gate_ns: u64 = samples.iter().map(|s| s.accent_gate_ns).sum();
    let accent_take_ns: u64 = samples
        .iter()
        .filter(|s| s.accent_taken)
        .map(|s| s.accent_rewrite_ns)
        .sum();
    let pua_ns: u64 = samples.iter().map(|s| s.pua_scan_ns).sum();

    let crlf_taken_ct = samples.iter().filter(|s| s.crlf_taken).count();
    let rule_taken_ct = samples.iter().filter(|s| s.rule_taken).count();
    let accent_taken_ct = samples.iter().filter(|s| s.accent_taken).count();

    println!("=== Phase 0 sub-pass breakdown (corpus aggregate) ===");
    println!(
        "documents     : {n}, total bytes {bytes} ({:.1} MB)",
        bytes as f64 / 1_000_000.0
    );
    println!("phase 0 total : {:.0} ms", total_ns as f64 / NS_PER_MS);
    println!();
    println!("--- per sub-pass ---");
    line("bom_strip", bom_ns, total_ns, bytes, "always");
    line(
        "crlf_gate (skip path)",
        crlf_gate_ns,
        total_ns,
        bytes,
        &format!("{} docs no \\r", n as usize - crlf_taken_ct),
    );
    line(
        "crlf_normalize",
        crlf_take_ns,
        total_ns,
        bytes,
        &format!("{crlf_taken_ct} docs"),
    );
    line(
        "rule_gate (has_long_rule_line)",
        rule_gate_ns,
        total_ns,
        bytes,
        "always",
    );
    line(
        "rule_isolate",
        rule_take_ns,
        total_ns,
        bytes,
        &format!("{rule_taken_ct} docs"),
    );
    line(
        "accent_gate (contains 〔)",
        accent_gate_ns,
        total_ns,
        bytes,
        "always",
    );
    line(
        "accent_rewrite",
        accent_take_ns,
        total_ns,
        bytes,
        &format!("{accent_taken_ct} docs"),
    );
    line(
        "pua_scan (memchr 0xEE)",
        pua_ns,
        total_ns,
        bytes,
        "always",
    );
}

fn line(name: &str, ns: u64, total: u64, bytes: u64, note: &str) {
    let pct = if total == 0 {
        0.0
    } else {
        ns as f64 / total as f64 * 100.0
    };
    let mb_per_s = if ns == 0 {
        0.0
    } else {
        bytes as f64 / (ns as f64 / 1e9) / 1_000_000.0
    };
    println!(
        "  {:32} : {:6.1}% ({:7.0} ms, {:7.0} MB/s)  [{note}]",
        name,
        pct,
        ns as f64 / NS_PER_MS,
        mb_per_s
    );
}
