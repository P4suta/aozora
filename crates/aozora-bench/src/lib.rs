//! Bench harness for the aozora parser.
//!
//! This crate hosts criterion benchmarks shared across the workspace
//! and is also the **canonical PGO profile source**: when the
//! release pipeline is configured to do PGO + BOLT
//! (`docs/adr/0009-clean-layered-architecture.md` Move 4 verification
//! plan), the profile collection step runs
//! `cargo run --release --bin aozora_pgo_train` against the full
//! corpus to gather an even sample of real-world parse work.
//!
//! ## Why a separate crate?
//!
//! - The benches need both `aozora` (the public API) and the
//!   internal `aozora-corpus` walker, neither of which fit in any
//!   one of the existing crates without inverting deps.
//! - Keeping bench harnesses together means we can run
//!   `cargo bench -p aozora-bench` and have one place to point CI's
//!   regression-detection workflow at.

#![forbid(unsafe_code)]

use std::hint::black_box;
use std::path::Path;

use aozora::Document;
use aozora_corpus::{CorpusItem, CorpusSource, FilesystemCorpus};
use aozora_encoding::decode_sjis;

/// Iterate the corpus rooted at `root`, decode SJIS bytes to UTF-8,
/// and parse each document.
///
/// Returns the `(decode_error_count, io_error_count, parsed_doc_count)`
/// triple. Used by the PGO training binary AND by the
/// synthetic-corpus criterion harness.
///
/// # Errors
///
/// Returns `Err` on a corpus-construction failure (typically: the
/// supplied root is not a directory). Per-file errors are counted
/// internally rather than raised — the goal is "exercise as much of
/// the parse path as possible" for profiling.
pub fn parse_corpus<P: AsRef<Path>>(
    root: P,
) -> Result<(usize, usize, usize), aozora_corpus::CorpusError> {
    let corpus = FilesystemCorpus::new(root.as_ref())?;
    let mut decode_errors = 0;
    let mut io_errors = 0;
    let mut parsed = 0;
    for item in corpus.iter() {
        match item {
            Ok(CorpusItem { bytes, .. }) => match decode_sjis(&bytes) {
                Ok(text) => {
                    let doc = Document::new(text.clone());
                    let tree = doc.parse();
                    // Touch the tree so the optimizer can't hoist
                    // the parse out — a real consumer reads
                    // diagnostics and the registry, both of which we
                    // surface here to match prod-shaped pressure.
                    let diag_count = tree.diagnostics().len();
                    black_box(diag_count);
                    parsed += 1;
                }
                Err(_) => decode_errors += 1,
            },
            Err(_) => io_errors += 1,
        }
    }
    Ok((decode_errors, io_errors, parsed))
}

/// Build a synthetic Aozora-shaped buffer of approximately
/// `target_bytes` size.
///
/// Mixes plain hiragana, kanji, ruby spans, and annotations so the
/// parser exercises every classifier path. Used by
/// `benches/synthetic_corpus.rs` to give a portable benchmark that
/// doesn't depend on a local corpus checkout.
#[must_use]
pub fn build_synthetic_aozora(target_bytes: usize) -> String {
    // One repeating unit covers each major construct exactly once.
    let unit = "明治の頃｜青梅《おうめ》街道沿いに、※［＃「木＋吶のつくり」、第3水準1-85-54］\
                なる珍しき木が立つ。［＃ここから2字下げ］その下で人々は語らひ。\
                ［＃ここで字下げ終わり］\n\n";
    let unit_bytes = unit.len();
    let cycles = target_bytes.div_ceil(unit_bytes);
    let mut s = String::with_capacity(cycles * unit_bytes);
    for _ in 0..cycles {
        s.push_str(unit);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_synthetic_returns_at_least_target_bytes() {
        let s = build_synthetic_aozora(1024);
        assert!(s.len() >= 1024);
    }

    #[test]
    fn build_synthetic_parses_without_panic() {
        let s = build_synthetic_aozora(4096);
        let doc = Document::new(s.clone());
        let tree = doc.parse();
        black_box(tree);
    }

    #[test]
    fn build_synthetic_emits_no_diagnostics_for_well_formed_input() {
        let s = build_synthetic_aozora(2048);
        let doc = Document::new(s.clone());
        let tree = doc.parse();
        assert!(
            tree.diagnostics().is_empty(),
            "synthetic corpus should be diagnostic-free, got {:?}",
            tree.diagnostics()
        );
    }
}
