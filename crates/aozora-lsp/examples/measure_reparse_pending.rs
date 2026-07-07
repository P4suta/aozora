//! Tier-2 end-to-end attribution for the LSP per-edit reparse path (#284,
//! T2-PR1 measure step). `incremental_speedup` (in `aozora-bench`) times the
//! *engine* in isolation on an already-sanitized LF buffer and reports 86×; it
//! deliberately excludes everything the LSP wraps around the engine. This
//! example measures those wrappers — the O(doc) passes that make the real
//! per-keystroke (debounced) reparse a multi-ms floor on the 100%-CRLF corpus —
//! and attributes each by re-running the primitive in isolation on the same
//! data.
//!
//! For every corpus document two variants are driven, so the LF/CRLF split is
//! explicit (the real corpus is CRLF + leading BOM; the engine bench is LF):
//!
//! - **LF** — the sanitize fixed point (`Document::lex().sanitized`),
//!   the engine's own coordinate space. The LSP's LF-clean fast path
//!   (`prior.sanitized == self.text`) applies, so no re-sanitize runs.
//! - **CRLF** — the LF text with `\n`→`\r\n` and a leading `U+FEFF`, the real
//!   aozora-bunko shape. The LSP's `try_incremental_crlf` path runs, which
//!   re-sanitizes the whole document every edit (verified here:
//!   `sanitize(crlf) == lf`, so it is a clean BOM-strip + CRLF→LF case).
//!
//! Per band (sanitized size) × variant, the timed columns isolate the passes:
//!
//! - `full`      — `Document::lex(edited)`: a from-scratch parse, the
//!   cost of any non-fast-path edit (the thing incremental beats).
//! - `concat`    — pass 1: rebuild a contiguous `String` from the paragraph
//!   ropes (what `reparse_pending` does before handing the engine a `&str`).
//! - `cache`     — passes 2/2c/3/4a: `ParseCache::reparse_incremental`, the
//!   LSP-side incremental splice. Includes the CRLF re-sanitize (pass 2), the
//!   engine's prefix/suffix memcmp + region re-lex (pass 3), and the base text
//!   copy (pass 4a). `fast %` = the share that took the splice (`cache_hits>0`).
//! - `sanitize`  — pass 2 alone: `sanitize(edited)` (the CRLF re-sanitize).
//! - `lineidx`   — pass 5: `LineIndex::new(edited)`, the publish-side line table
//!   the diagnostic position mapping builds over the whole raw text each
//!   publish.
//! - `engine`    — the floor the rope work targets: the pure diagnostics-only
//!   engine (`reparse_incremental_diagnostics_only` over the maintained
//!   `PieceSeq`) on the LF buffer, exactly the `incremental_speedup` `diag`
//!   column. Everything above `engine` is O(doc) overhead the LSP adds on top.
//!
//! `e2e ≈ concat + cache + lineidx` is the per-edit LSP floor; `e2e − engine`
//! is the O(doc) overhead Tier-2 rope-native removes. Only fast-path docs feed
//! the means (a fallback's incremental cost is just `full`). Not a gate — run
//! once and record the bands in the PR.
//!
//! ```text
//! AOZORA_CORPUS_ROOT=… cargo run -p aozora-lsp --release --features internals \
//!     --example measure_reparse_pending
//! ```

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::disallowed_methods,
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::needless_range_loop,
    reason = "profiling tool, not library code"
)]

use std::hint::black_box;
use std::process;
use std::slice;
use std::time::Instant;

use aozora::pipeline::has_long_rule_line;
use aozora::pipeline::lexer::sanitize::sanitize;
use aozora::{DiagBaseRef, Document, PieceSeq, reparse_incremental_diagnostics_only};
use aozora_encoding::decode_auto;
use aozora_lsp::internals::{ByteEdit, LineIndex, ParseCache, apply_edits};
use ropey::Rope;

/// Size bands in bytes (sanitized length): `[lo, hi)` — same split as
/// `incremental_speedup` so the engine floor is directly comparable.
const BANDS: &[(&str, u64, u64)] = &[
    ("< 50 KiB", 0, 50 * 1024),
    ("50 KiB – 500 KiB", 50 * 1024, 500 * 1024),
    ("500 KiB – 2 MiB", 500 * 1024, 2 * 1024 * 1024),
    ("> 2 MiB", 2 * 1024 * 1024, u64::MAX),
];

fn band_of(len: u64) -> usize {
    BANDS
        .iter()
        .position(|&(_, lo, hi)| len >= lo && len < hi)
        .unwrap_or(0)
}

/// Per-(band, variant) accumulators. Times are summed over fast-path docs.
#[derive(Default, Clone, Copy)]
struct Acc {
    docs: u64,
    fast: u64,
    full_ns: u128,
    concat_ns: u128,
    cache_ns: u128,
    sanitize_ns: u128,
    lineidx_ns: u128,
    engine_ns: u128,
    /// Why a non-fast-path edit fell back to a full parse (CRLF only): a
    /// whole-document long rule line declined by `has_long_rule_line` (the
    /// suspected dominant cause), a post-edit sanitize diagnostic, or the
    /// engine prologue itself (structural decline — same cause as the LF path).
    decline_rule: u64,
    decline_sanitize: u64,
    decline_engine: u64,
}

/// One char-boundary byte offset near the middle of `s` that is *mid-line*
/// (neither it nor the byte before it is `\r`/`\n`), so an inserted `x` cannot
/// split a `\r\n` pair or land at a line edge. Returns `None` if no such
/// position exists (e.g. a doc that is all blank lines) — that doc is skipped
/// for the variant.
fn mid_line_boundary(s: &str) -> Option<usize> {
    let start = s.len() / 2;
    for i in start..s.len() {
        if !s.is_char_boundary(i) {
            continue;
        }
        let before = s.as_bytes().get(i.wrapping_sub(1)).copied();
        let at = s.as_bytes().get(i).copied();
        let edge = |b: Option<u8>| matches!(b, Some(b'\r' | b'\n'));
        if i > 0 && !edge(before) && !edge(at) {
            return Some(i);
        }
    }
    None
}

/// Build the real corpus shape from an LF fixed point: prepend a BOM and turn
/// every `\n` into `\r\n`. Returns `None` unless this is a clean BOM-strip +
/// CRLF→LF case (`sanitize` reproduces `lf` with no diagnostic), so the CRLF
/// measurement reflects the LSP's actual `try_incremental_crlf` fast path.
fn crlf_variant(lf: &str) -> Option<String> {
    let mut s = String::with_capacity(lf.len() + lf.len() / 16 + 3);
    s.push('\u{FEFF}');
    for ch in lf.chars() {
        if ch == '\n' {
            s.push('\r');
        }
        s.push(ch);
    }
    let san = sanitize(&s);
    (san.diagnostics.is_empty() && &*san.text == lf).then_some(s)
}

fn main() {
    let Some(corpus) = aozora_corpus::from_env() else {
        println!("measure_reparse_pending: AOZORA_CORPUS_ROOT not set — nothing to measure.");
        process::exit(0);
    };

    // Decode + sanitize up front; keep only sanitize fixed points (the engine
    // assumes a stable sanitized baseline), exactly like `incremental_speedup`.
    let mut docs: Vec<String> = Vec::new();
    for item in corpus.iter().filter_map(Result::ok) {
        let Ok(text) = decode_auto(&item.bytes) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let san = Document::new(text.as_ref()).lex().sanitized;
        if san.is_empty() {
            continue;
        }
        if Document::new(san.as_str()).lex().sanitized != san {
            continue;
        }
        docs.push(san);
    }
    if docs.is_empty() {
        println!("measure_reparse_pending: corpus yielded 0 usable documents.");
        process::exit(0);
    }
    eprintln!(
        "measure_reparse_pending: {} docs ready, measuring LF + CRLF…",
        docs.len()
    );

    // [band][variant] where variant 0 = LF, 1 = CRLF.
    let mut acc = [[Acc::default(); 2]; BANDS.len()];

    for lf in &docs {
        let b = band_of(lf.len() as u64);
        // The engine floor is variant-independent (it works in sanitized LF
        // coordinates); compute it once per doc and credit both variants.
        let engine_ns = measure_engine_floor(lf);

        if let Some(mid) = mid_line_boundary(lf) {
            measure_variant(&mut acc[b][0], lf, mid, false, engine_ns);
        }
        if let Some(crlf) = crlf_variant(lf)
            && let Some(mid) = mid_line_boundary(&crlf)
        {
            measure_variant(&mut acc[b][1], &crlf, mid, true, engine_ns);
        }
    }

    report(&acc);
}

/// The pure diagnostics-only engine on the LF buffer — the floor everything
/// above which is LSP-side O(doc) overhead. Mirrors `incremental_speedup`'s
/// `diag` column: the maintained `PieceSeq` is built once outside the timer
/// (production maintains it across edits, never rebuilding it per edit), so the
/// timer measures only the splice.
fn measure_engine_floor(lf: &str) -> u128 {
    let Some(mid) = mid_line_boundary(lf) else {
        return 0;
    };
    let cached = Document::new(lf).lex();
    let new_lf = format!("{}x{}", &lf[..mid], &lf[mid..]);
    black_box(Document::new(new_lf.as_str()).lex());
    let pieces = PieceSeq::from_contiguous(
        &cached.source_nodes,
        &cached.pairs,
        &cached.diagnostics,
        cached.sanitized_len,
    );

    let t = Instant::now();
    let diag = reparse_incremental_diagnostics_only(
        DiagBaseRef::from_cached(&cached, &pieces),
        &new_lf,
        mid..mid,
    );
    let ns = t.elapsed().as_nanos();
    black_box(diag);
    ns
}

/// Drive one variant (`v` = the raw editor buffer) through the LSP-side passes
/// for a single mid-line `x` insertion at `mid`, accumulating into `a`. Only a
/// document that takes the incremental splice (`cache_hits > 0`) feeds the
/// timed means; every document counts toward `docs` and the fast-path rate.
fn measure_variant(a: &mut Acc, v: &str, mid: usize, is_crlf: bool, engine_ns: u128) {
    a.docs += 1;

    let edit = ByteEdit::new(mid..mid, "x".to_owned());
    let Ok(edited) = apply_edits(v, slice::from_ref(&edit)) else {
        return;
    };

    // Seed the cache with a full parse of the original buffer (so the next
    // call is the warm incremental path), then warm the edited text's pages.
    let mut cache = ParseCache::default();
    let (_seed_diags, _seed_stats) = cache.reparse(v);
    black_box(Document::new(edited.as_str()).lex());

    // Pass 1 — rope→String concat (what reparse_pending does each reparse).
    let rope = Rope::from(edited.as_str());
    let t = Instant::now();
    let mut concat = String::with_capacity(rope.len_bytes());
    for chunk in rope.chunks() {
        concat.push_str(chunk);
    }
    let concat_ns = t.elapsed().as_nanos();
    black_box(&concat);

    // Passes 2/2c/3/4a — the LSP-side incremental splice; stats tell fast-path.
    // Feeds the post-edit rope straight in (Mechanism B: the cache splices its
    // sanitized rope, no per-keystroke `to_string` + `sanitize`).
    let t = Instant::now();
    let (_diags, stats) = cache.reparse_incremental(&rope, slice::from_ref(&edit));
    let cache_ns = t.elapsed().as_nanos();
    let fast = stats.cache_hits > 0;

    // Pass 2 alone — the CRLF re-sanitize (run for both variants for contrast).
    let t = Instant::now();
    black_box(sanitize(&edited));
    let sanitize_ns = t.elapsed().as_nanos();

    // Pass 5 — the publish-side line-index build over the whole raw text.
    let t = Instant::now();
    black_box(LineIndex::new(&edited));
    let lineidx_ns = t.elapsed().as_nanos();

    // full — the fallback cost (what a non-fast-path edit pays).
    let t = Instant::now();
    black_box(Document::new(edited.as_str()).lex());
    let full_ns = t.elapsed().as_nanos();

    if fast {
        a.fast += 1;
        a.full_ns += full_ns;
        a.concat_ns += concat_ns;
        a.cache_ns += cache_ns;
        a.sanitize_ns += sanitize_ns;
        a.lineidx_ns += lineidx_ns;
        a.engine_ns += engine_ns;
    } else if is_crlf {
        // Categorize why the real (CRLF) path declined. The same document
        // fast-paths far more often as LF, so the gap is the CRLF-specific
        // gates in `try_incremental_crlf` (whole-doc rule line, post-edit
        // sanitize diagnostic), versus a structural engine decline that the
        // LF path hits too.
        if has_long_rule_line(&edited) {
            a.decline_rule += 1;
        } else if !sanitize(&edited).diagnostics.is_empty() {
            a.decline_sanitize += 1;
        } else {
            a.decline_engine += 1;
        }
    }
}

fn report(acc: &[[Acc; 2]; BANDS.len()]) {
    println!("=== measure_reparse_pending (LSP per-edit O(doc) attribution) ===\n");
    println!(
        "{:<17} {:<5} {:>6} {:>6} {:>9} {:>8} {:>8} {:>9} {:>8} {:>8} {:>8}",
        "band",
        "var",
        "docs",
        "fast%",
        "full µs",
        "concat",
        "cache",
        "sanitize",
        "lineidx",
        "engine",
        "e2e µs",
    );
    let mut tot = [Acc::default(); 2];
    for row in acc {
        for (vi, a) in row.iter().enumerate() {
            tot[vi].add(a);
        }
    }
    for (i, &(name, _, _)) in BANDS.iter().enumerate() {
        for vi in 0..2 {
            let a = &acc[i][vi];
            if a.docs == 0 {
                continue;
            }
            print_row(name, vi, a);
        }
    }
    println!("{:-<108}", "");
    for vi in 0..2 {
        if tot[vi].docs > 0 {
            print_row("all", vi, &tot[vi]);
        }
    }
    println!(
        "\nµs = mean per fast-path doc. e2e = concat+cache+lineidx (the per-edit LSP floor); \
         engine = the diagnostics-only engine alone (incremental_speedup's diag column). \
         e2e − engine is the O(doc) overhead Tier-2 rope-native removes; for CRLF the \
         `sanitize` column is the dominant part of it. var: LF = sanitize fixed point \
         (LF-clean fast path); CRLF = BOM + \\r\\n (the real corpus, try_incremental_crlf path)."
    );

    // Why the real (CRLF) path declines — the coverage story. If `rule`
    // dominates, the whole-document `has_long_rule_line` gate (every aozora
    // file's header carries a long `----` rule) is what kills the CRLF fast
    // path; an edit-local trigger gate over a maintained sanitized rope
    // (Tier-2) recovers it without re-checking the whole document.
    println!("\n=== CRLF fall-back reasons (non-fast-path docs) ===");
    println!(
        "{:<17} {:>8} {:>8} {:>9} {:>8}",
        "band", "rule", "sanitize", "engine", "fast"
    );
    let mut tr = Acc::default();
    for (i, &(name, _, _)) in BANDS.iter().enumerate() {
        let a = &acc[i][1];
        if a.docs == 0 {
            continue;
        }
        tr.add(a);
        println!(
            "{name:<17} {:>8} {:>8} {:>9} {:>8}",
            a.decline_rule, a.decline_sanitize, a.decline_engine, a.fast
        );
    }
    println!("{:-<54}", "");
    println!(
        "{:<17} {:>8} {:>8} {:>9} {:>8}",
        "all", tr.decline_rule, tr.decline_sanitize, tr.decline_engine, tr.fast
    );
}

impl Acc {
    fn add(&mut self, o: &Self) {
        self.docs += o.docs;
        self.fast += o.fast;
        self.full_ns += o.full_ns;
        self.concat_ns += o.concat_ns;
        self.cache_ns += o.cache_ns;
        self.sanitize_ns += o.sanitize_ns;
        self.lineidx_ns += o.lineidx_ns;
        self.engine_ns += o.engine_ns;
        self.decline_rule += o.decline_rule;
        self.decline_sanitize += o.decline_sanitize;
        self.decline_engine += o.decline_engine;
    }
}

fn print_row(name: &str, vi: usize, a: &Acc) {
    let var = if vi == 0 { "LF" } else { "CRLF" };
    let fast_pct = 100.0 * a.fast as f64 / a.docs.max(1) as f64;
    let n = a.fast.max(1) as f64;
    let us = |ns: u128| ns as f64 / n / 1000.0;
    let concat = us(a.concat_ns);
    let cache = us(a.cache_ns);
    let lineidx = us(a.lineidx_ns);
    let e2e = concat + cache + lineidx;
    println!(
        "{name:<17} {var:<5} {:>6} {fast_pct:>5.1}% {:>9.1} {concat:>8.1} {cache:>8.1} \
         {:>9.1} {lineidx:>8.1} {:>8.1} {e2e:>8.1}",
        a.docs,
        us(a.full_ns),
        us(a.sanitize_ns),
        us(a.engine_ns),
    );
}
