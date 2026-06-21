//! One-shot wall-time measurement of the incremental snapshot
//! rebuild on a real-world large document.
//!
//! Loads `samples/bouten.afm` (6.3 MB, 24 k gaiji), opens a
//! `OpenDocument`, and times two single-character edits at different
//! positions to compare cold vs incremental rebuild paths:
//!
//! - Edit 1 (cold path)            — at offset 0, every byte shifts.
//!   Tree-sitter's `changed_ranges` covers the whole doc, so the
//!   incremental algorithm degenerates to full re-walk.
//! - Edit 2 (incremental fast path) — at the doc midpoint, on a UTF-8
//!   boundary. Only a small region's gaiji spans need re-extraction;
//!   the surrounding ~24 k spans pass through with cumulative-delta
//!   shift.
//!
//! Run with:
//! ```text
//! cargo run -p aozora-lsp --release --example measure_incremental
//! ```

use std::env;
use std::fs;
use std::path::Path;
use std::time::Instant;

use aozora_lsp::internals::{ByteEdit, OpenDocument};

/// Synthesise a gaiji-rich document by repeating a `※[#…]` block.
/// `count` blocks → roughly `count * 60` bytes of gaiji content +
/// surrounding plain prose. Keeps tree-sitter's `gaiji` rule
/// exercised so the incremental algorithm has spans to carry forward.
fn synth_gaiji_doc(count: usize) -> String {
    let mut s = String::with_capacity(count * 80);
    for i in 0..count {
        s.push('第');
        s.push_str(&i.to_string());
        s.push_str("章 一行の文章です。");
        s.push_str("※［＃「あ」、第3水準1-85-54］という外字。");
        s.push('\n');
    }
    s
}

/// Bytes → tenths-of-MiB pair `(whole, tenths)` for the diagnostic
/// printouts. Using integer arithmetic instead of an `as f64` cast
/// keeps the display lossless for any document size that fits in
/// `usize` (overflow only at exabyte scale, well past anything
/// realistic) and avoids `clippy::cast_precision_loss`.
const fn bytes_mb_tenths(n: usize) -> (usize, usize) {
    // Round-half-to-even on the tenths boundary by computing in
    // tenths and dividing once: `n * 10 / MiB` gives the tenths
    // value with truncation; that matches the prior `{:.1}` printf
    // output to the same digit (printf truncates, doesn't round,
    // when the next digit is < 5 — same here).
    const MIB: usize = 1_048_576;
    let tenths_total = (n / MIB * 10) + (n % MIB * 10 / MIB);
    (tenths_total / 10, tenths_total % 10)
}

fn measure_corpus_doc() {
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let path = Path::new(&manifest)
        .join("../..")
        .join("samples")
        .join("bouten.afm");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let (mb, tenths) = bytes_mb_tenths(text.len());
    println!(
        "loaded fixture: {} bytes from {} ({mb}.{tenths} MiB)",
        text.len(),
        path.display(),
    );

    // Cold-start cost (OpenDocument::new)
    let t = Instant::now();
    let state = OpenDocument::new(text);
    println!("OpenDocument::new: {:?}", t.elapsed());

    let initial_spans = state.snapshot().doc_gaiji_spans().len();
    println!("initial gaiji spans: {initial_spans}");

    // Edit at offset 0 — worst case for incremental TS reparse and
    // for changed_ranges (everything shifts).
    let t = Instant::now();
    state
        .apply_changes(&[ByteEdit::new(0..0, " ".to_owned())])
        .unwrap();
    println!("apply_changes (offset 0, cold path): {:?}", t.elapsed());
    let v1_spans = state.snapshot().doc_gaiji_spans().len();
    println!("post-edit-1 gaiji spans: {v1_spans}");

    // Mid-document edit — the path designed for the incremental
    // algorithm. UTF-8-safe boundary at half the doc length.
    let snap = state.snapshot();
    let mid_target = snap.doc_text().len() / 2;
    drop(snap);
    let mid_offset = nearest_char_boundary(&state, mid_target);

    let t = Instant::now();
    state
        .apply_changes(&[ByteEdit::new(mid_offset..mid_offset, " ".to_owned())])
        .unwrap();
    println!(
        "apply_changes (mid-doc @ {mid_offset}, incremental path): {:?}",
        t.elapsed()
    );
    let v2_spans = state.snapshot().doc_gaiji_spans().len();
    println!("post-edit-2 gaiji spans: {v2_spans}");

    // Bench another mid-doc edit so caches stay warm and we have a
    // second data point. Snap the next offset to a UTF-8 boundary —
    // a naïve `mid_offset + 1` lands inside a multi-byte char on
    // Japanese-heavy fixtures and `apply_changes` rejects it as a
    // `NonCharBoundary` edit, so the .unwrap() panics.
    let next_offset = nearest_char_boundary(&state, mid_offset + 1);
    let t = Instant::now();
    state
        .apply_changes(&[ByteEdit::new(next_offset..next_offset, " ".to_owned())])
        .unwrap();
    println!(
        "apply_changes (mid-doc @ {next_offset} again, incremental path): {:?}",
        t.elapsed()
    );
}

fn main() {
    measure_corpus_doc();
    // Synthetic gaiji-rich document so the incremental algorithm has
    // spans to carry forward. bouten.afm has zero gaiji (it's all
    // 傍点 annotations, a different node kind), which masks the
    // potential win — the cost reduction only manifests when there
    // is a non-empty old_spans set to skip re-extracting.
    println!("\n--- synthetic gaiji-rich doc ---");
    let gaiji_text = synth_gaiji_doc(50_000);
    let (mb, tenths) = bytes_mb_tenths(gaiji_text.len());
    println!("synth doc: {} bytes ({mb}.{tenths} MiB)", gaiji_text.len());

    let t = Instant::now();
    let g_state = OpenDocument::new(gaiji_text);
    println!("  OpenDocument::new: {:?}", t.elapsed());
    let g_initial = g_state.snapshot().doc_gaiji_spans().len();
    println!("  initial gaiji spans: {g_initial}");

    // Cold path on synth doc
    let t = Instant::now();
    g_state
        .apply_changes(&[ByteEdit::new(0..0, " ".to_owned())])
        .unwrap();
    println!(
        "  apply_changes (offset 0, cold-degenerate): {:?}",
        t.elapsed()
    );

    // Mid-doc edit — incremental path should bypass walking 49 999
    // unchanged gaiji blocks.
    let snap = g_state.snapshot();
    let mid = nearest_char_boundary(&g_state, snap.doc_text().len() / 2);
    drop(snap);

    let t = Instant::now();
    g_state
        .apply_changes(&[ByteEdit::new(mid..mid, " ".to_owned())])
        .unwrap();
    println!(
        "  apply_changes (mid-doc @ {mid}, incremental path): {:?}",
        t.elapsed()
    );

    // Same UTF-8 boundary snap as the bouten path above — `mid + 1`
    // can land inside a multi-byte char on a kanji-heavy synthetic.
    let mid_next = nearest_char_boundary(&g_state, mid + 1);
    let t = Instant::now();
    g_state
        .apply_changes(&[ByteEdit::new(mid_next..mid_next, " ".to_owned())])
        .unwrap();
    println!(
        "  apply_changes (mid-doc @ {mid_next} again, incremental path): {:?}",
        t.elapsed()
    );
    let g_final = g_state.snapshot().doc_gaiji_spans().len();
    println!("  post-edits gaiji spans: {g_final}");
}

fn nearest_char_boundary(state: &OpenDocument, target: usize) -> usize {
    let snap = state.snapshot();
    let mut idx = target.min(snap.doc_text().len());
    while idx > 0 && !snap.doc_text().is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}
