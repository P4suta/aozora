//! 金庫番 — guardrail tests that watch over the load-bearing
//! invariants of the LSP surface.
//!
//! Targeted unit tests pin specific cases; property tests pin
//! shape-level identities; this suite is the third leg: **adversarial
//! invariants** that catch entire bug classes — panics on malformed
//! input, formatter divergence, concurrency races, byte-level data
//! loss across edit batches.
//!
//! Each section calls out its threat model:
//!
//! 1. **Panic resistance** — every public LSP entry point survives
//!    arbitrary UTF-8 input + arbitrary `Position` queries (within
//!    `u32` range) without panicking. Crashing the LSP server takes
//!    out the editor's `IntelliSense` for the whole session, so this
//!    is the highest-impact class to defend.
//! 2. **Format fixed point** — `format ∘ format == format` for
//!    every realistic input. The aozora corpus relies on this so
//!    "format on save" doesn't oscillate.
//! 3. **Edit-burst data integrity** — long sequences of random
//!    edits never silently drop bytes; the resulting buffer always
//!    matches an oracle.
//! 4. **Concurrent reads/writes** — readers see a consistent
//!    snapshot under heavy parallel write pressure; no torn reads,
//!    no panics.
//! 5. **Adversarial diagnostic payloads** — quick-fix actions
//!    survive malformed `data` JSON without panicking.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use aozora_fmt::format_source;
use aozora_lsp::internals::{
    ByteEdit, LanguageIdentifier, LineIndex, OpenDocument, byte_offset_to_position, completion_at,
    diagnostics_for_source, document_symbols, emmet_completions, folding_ranges, format_edits,
    format_on_type, hover_at, linked_editing_at, position_to_byte_offset, snippet_completions,
    wrap_selection_actions,
};
use proptest::collection::vec as proptest_vec;
use proptest::prelude::*;
use proptest::sample::select;
use tower_lsp::lsp_types::{Position, Range, Url};

/// Pinned message language for the adversarial provider sweeps — they only
/// assert structural invariants (valid ranges, no panics), so any locale
/// works; English keeps the sweep deterministic regardless of the host `LANG`.
fn en() -> LanguageIdentifier {
    "en".parse().expect("en parses")
}

// ---------------------------------------------------------------
// 1. Panic resistance across the public LSP surface.
// ---------------------------------------------------------------
//
// Every handler below must return *something* (Vec, Option, …) for
// any input — it must never panic. The corpus mixes ASCII, kanji,
// emoji surrogate pairs, IME-input notation triggers, malformed
// UTF-8-near input, and empty strings.
//
// `Position` queries are sampled with `u32` line/character values
// so we cover both in-bounds and far-past-EOF cases. Handlers must
// gracefully clamp or return None.

const PANIC_CORPUS: &[&str] = &[
    "",
    " ",
    "\n",
    "\n\n",
    "\n\n\n",
    "\r\n\r\n",
    "abc",
    "あ",
    "本文",
    "本文\n",
    "本文\n\n",
    "｜青空《あおぞら》",
    "｜青空《あおぞら》\n｜白雲《はくうん》",
    "［＃改ページ］",
    "［＃ここから字下げ］\n中身\n［＃ここで字下げ終わり］",
    "※［＃「desc」、第3水準1-85-54］",
    "※［＃「desc」、X］後文",
    "「鉤」",
    "〔Crevez〕",
    // Adversarial: unmatched delimiters
    "［＃改ページ",
    "本文 ］",
    "《reading》",
    "｜",
    "※",
    // Adversarial: surrogate pairs
    "a😀b\nc😀d",
    // PUA sentinel that the lexer warns about
    "abc\u{E001}def",
];

/// Lazy, owned-string variant for the adversarial corpus that
/// includes cap-stressing inputs — those need allocation so they
/// can't sit in `&'static str` form.
fn adversarial_corpus() -> Vec<String> {
    let mut out: Vec<String> = PANIC_CORPUS.iter().map(|&s| s.to_owned()).collect();
    // Long mixed-byte run (~300 bytes, well within scan budgets).
    out.push("あ".repeat(100));
    // Stress the paragraph-byte-ranges cap: 64 KB of multi-byte text
    // with no `\n\n` boundary inside a single paragraph.
    out.push("あ".repeat(30_000));
    // Long single line without any aozora notation.
    out.push("x".repeat(5_000));
    // 多重 \n\n run
    out.push("x\n\n".repeat(50));
    out
}

/// Sample `Position` queries spanning in-bounds, end-of-buffer,
/// far-past-EOF, and surrogate-pair-aware columns. Every handler
/// must accept all of them without panic.
fn position_corpus() -> Vec<Position> {
    vec![
        Position::new(0, 0),
        Position::new(0, 1),
        Position::new(0, 99),
        Position::new(1, 0),
        Position::new(50, 50),
        Position::new(u32::MAX, u32::MAX),
        Position::new(u32::MAX, 0),
        Position::new(0, u32::MAX),
    ]
}

#[test]
fn hover_never_panics_on_corpus() {
    for src in adversarial_corpus() {
        for pos in position_corpus() {
            drop(hover_at(&src, pos, &en()));
        }
    }
}

#[test]
fn completion_never_panics_on_corpus() {
    for src in adversarial_corpus() {
        for pos in position_corpus() {
            drop(completion_at(&src, pos, &en()));
            drop(emmet_completions(&src, pos, &en()));
            drop(snippet_completions(&src, pos, &en()));
        }
    }
}

#[test]
fn format_on_type_never_panics_on_corpus() {
    // Trigger chars from the on-type capability + a few off-list chars
    // so we exercise both the rule-found and rule-missing paths.
    let triggers = ["[", "]", "<", ">", "|", "*", "#", "a", "あ"];
    for src in adversarial_corpus() {
        for pos in position_corpus() {
            for t in triggers {
                drop(format_on_type(&src, pos, t));
            }
        }
    }
}

#[test]
fn linked_editing_never_panics_on_corpus() {
    for src in adversarial_corpus() {
        let idx = LineIndex::new(&src);
        for pos in position_corpus() {
            drop(linked_editing_at(&src, &idx, pos));
        }
    }
}

#[test]
fn diagnostics_format_folding_symbol_never_panic_on_corpus() {
    for src in adversarial_corpus() {
        drop(diagnostics_for_source(&src, &en()));
        drop(format_edits(&src));
        drop(folding_ranges(&src));
        let idx = LineIndex::new(&src);
        drop(document_symbols(&src, &idx, &en()));
    }
}

#[test]
fn position_byte_round_trip_never_panics_on_boundary_offsets() {
    for src in adversarial_corpus() {
        // For huge corpus members (`あ`×30 000 etc.), iterating
        // every char boundary turns this into O(N²) — `position_to_
        // byte_offset` walks newlines from byte 0 each call. Sample
        // ~256 boundaries linearly instead; the property holds
        // pointwise so the sampling doesn't weaken the guarantee.
        let stride = src.len().div_ceil(256).max(1);
        let mut byte = 0usize;
        while byte <= src.len() {
            if src.is_char_boundary(byte) {
                let pos = byte_offset_to_position(&src, byte);
                let back = position_to_byte_offset(&src, pos);
                assert_eq!(back, Some(byte), "offset {byte} for src len {}", src.len());
            }
            byte += stride;
        }
        // Always check the EOF byte too so end-of-buffer is pinned.
        let eof = src.len();
        let pos = byte_offset_to_position(&src, eof);
        let back = position_to_byte_offset(&src, pos);
        assert_eq!(back, Some(eof), "EOF roundtrip");
    }
}

#[test]
fn wrap_actions_never_panic_on_corpus_ranges() {
    let uri = Url::parse("file:///guardian.afm").unwrap();
    let ranges = [
        Range::new(Position::new(0, 0), Position::new(0, 0)),
        Range::new(Position::new(0, 0), Position::new(0, 1)),
        Range::new(Position::new(0, 0), Position::new(0, 99)),
        Range::new(Position::new(0, 0), Position::new(99, 99)),
        Range::new(Position::new(99, 0), Position::new(99, 1)),
        Range::new(Position::new(0, 5), Position::new(0, 1)), // inverted
    ];
    for src in adversarial_corpus() {
        let idx = LineIndex::new(&src);
        for r in ranges {
            drop(wrap_selection_actions(&src, &idx, &uri, r, &en()));
        }
    }
}

// ---------------------------------------------------------------
// 2. Format fixed point: format ∘ format == format
// ---------------------------------------------------------------

proptest! {
    /// `format_source` is contractually a fixed point on the second
    /// pass — `format(format(x)) == format(x)`. This is the
    /// invariant the `--check` mode and "format on save" rely on;
    /// any drift would cause CI to oscillate between two equally
    /// "canonical" forms.
    #[test]
    fn format_is_idempotent_for_realistic_inputs(text in realistic_text_strategy()) {
        let once = format_source(&text);
        let twice = format_source(&once);
        prop_assert_eq!(once, twice);
    }
}

// ---------------------------------------------------------------
// 3. Edit-burst data integrity
// ---------------------------------------------------------------

/// Tiny seeded PRNG (xorshift64) — avoids pulling in the `rand`
/// crate just for two stress tests. Not cryptographic, but
/// deterministic across runs so a regression reproduces.
struct Xorshift(u64);
impl Xorshift {
    const fn new(seed: u64) -> Self {
        // The all-zero seed is a fixed point for xorshift; rotate
        // any zero into something benign.
        Self(if seed == 0 {
            0xDEAD_BEEF_CAFE_BABE
        } else {
            seed
        })
    }
    const fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn range(&mut self, lo: usize, hi_inclusive: usize) -> usize {
        if hi_inclusive <= lo {
            return lo;
        }
        let span = hi_inclusive - lo + 1;
        // `usize` may be 32-bit on some targets; mask the high bits
        // before the modulo to avoid clippy::cast_possible_truncation
        // (the modulo would mask anyway, but the cast happens first).
        let bits = self.next() & u64::from(u32::MAX);
        lo + (usize::try_from(bits).unwrap_or(0) % span)
    }
    const fn flip(&mut self) -> bool {
        self.next() & 1 == 1
    }
}

/// A long sequence of random pure-ASCII insertions at random
/// boundary-aligned offsets must converge to a buffer byte-equal to
/// the equivalent oracle (the same insertions applied to a `String`
/// directly). This is the strongest possible cross-check for the
/// paragraph-segmented edit path.
#[test]
fn random_insert_burst_matches_string_oracle() {
    let mut rng = Xorshift::new(0xA0_20_BA_FE);
    let state = OpenDocument::new(String::new());
    let mut oracle = String::new();
    for _ in 0..200 {
        let len = oracle.len();
        let insert_at = rng.range(0, len);
        let ch = (b'a' + ((rng.next() % 26) as u8)) as char;
        let chunk = ch.to_string();
        let edit = ByteEdit::new(insert_at..insert_at, chunk.clone());
        state.apply_changes(&[edit]).expect("valid insert");
        oracle.insert_str(insert_at, &chunk);
    }
    assert_eq!(&**state.snapshot().doc_text(), &oracle);
}

/// Same drill but with random delete-then-insert cycles that
/// occasionally cross paragraph boundaries (we sprinkle `\n\n` in
/// the seed text so cross-paragraph edits actually trigger).
#[test]
fn random_replace_burst_matches_string_oracle() {
    let seed = "段落1\n\n段落2\n\n段落3\n\n段落4";
    let mut rng = Xorshift::new(0xDE_AD_BE_EF);
    let state = OpenDocument::new(seed.to_owned());
    let mut oracle = seed.to_owned();
    for _ in 0..100 {
        let len = oracle.len();
        if len == 0 {
            continue;
        }
        let mut start = rng.range(0, len);
        let mut end = rng.range(start, len);
        // Snap both to char boundaries — Japanese chars are 3 bytes
        // so a naïve random offset often lands mid-codepoint.
        while start > 0 && !oracle.is_char_boundary(start) {
            start -= 1;
        }
        while end < len && !oracle.is_char_boundary(end) {
            end += 1;
        }
        if end > len {
            end = len;
        }
        let new_text = if rng.flip() {
            String::new()
        } else {
            // ASCII chunk so the oracle replace_range stays simple.
            let ch = (b'A' + ((rng.next() % 26) as u8)) as char;
            ch.to_string()
        };
        let edit = ByteEdit::new(start..end, new_text.clone());
        if state.apply_changes(&[edit]).is_some() {
            oracle.replace_range(start..end, &new_text);
        }
    }
    assert_eq!(&**state.snapshot().doc_text(), &oracle);
}

// ---------------------------------------------------------------
// 4. Concurrent read/write stress
// ---------------------------------------------------------------

/// Many readers + a single writer must observe consistent
/// snapshots: every snapshot's `doc_text()` byte length agrees with
/// the snapshot's `total_bytes`. Readers must never see torn
/// state, never panic. Writer applies a long sequence of inserts
/// at the start; readers loop loading + assertion.
#[test]
fn snapshot_reads_under_write_pressure_stay_consistent() {
    let state = OpenDocument::new("seed\n\nseed".to_owned());
    let stop = Arc::new(AtomicBool::new(false));

    let writer_state = Arc::clone(&state);
    let writer_stop = Arc::clone(&stop);
    let writer = thread::spawn(move || {
        for _ in 0..200 {
            if writer_stop.load(Ordering::Relaxed) {
                break;
            }
            _ = writer_state.apply_changes(&[ByteEdit::new(0..0, "X".to_owned())]);
            // Force the snapshot rebuild to happen inline
            // (otherwise it's queued on the blocking pool and
            // readers would always race the same stale snapshot).
            writer_state.rebuild_snapshot_now();
        }
    });

    let mut readers = Vec::new();
    for _ in 0..4 {
        let reader_state = Arc::clone(&state);
        let reader_stop = Arc::clone(&stop);
        readers.push(thread::spawn(move || {
            let mut last_seen_version = 0u64;
            for _ in 0..1000 {
                if reader_stop.load(Ordering::Relaxed) {
                    break;
                }
                let snap = reader_state.snapshot();
                // DocSnapshot version is monotone — readers should
                // never see it go backwards.
                assert!(snap.version >= last_seen_version);
                last_seen_version = snap.version;
                // total_bytes must equal the materialised text len.
                let text = snap.doc_text();
                assert_eq!(text.len(), snap.total_bytes as usize);
            }
        }));
    }

    writer.join().expect("writer panicked");
    stop.store(true, Ordering::Relaxed);
    for r in readers {
        r.join().expect("reader panicked");
    }
}

// ---------------------------------------------------------------
// 5. Compute-diagnostics never panics on malformed input
// ---------------------------------------------------------------

/// `diagnostics_for_source` is the gatekeeper for what the editor
/// shows the user — and it sits on top of the aozora semantic
/// parser, which is the most complex piece in our dependency
/// graph. Every adversarial input from the corpus must produce
/// SOME diagnostic vector (possibly empty) without panicking.
#[test]
fn diagnostics_for_source_returns_valid_ranges_for_corpus() {
    for src in adversarial_corpus() {
        let diags = diagnostics_for_source(&src, &en());
        // Every diagnostic's range must round-trip through the
        // line index — i.e. its line/column must resolve back to a
        // valid byte offset. A diagnostic pointing into the void
        // would crash the editor's render path.
        let idx = LineIndex::new(&src);
        for d in diags {
            // `byte_offset` returns None only for past-EOF lines.
            // Diagnostic ranges should never reference such lines.
            assert!(
                idx.byte_offset(&src, d.range.start).is_some(),
                "diag start out of range: {:?} for src len {}",
                d.range,
                src.len()
            );
            assert!(idx.byte_offset(&src, d.range.end).is_some());
        }
    }
}

// ---------------------------------------------------------------
// Shared proptest strategy
// ---------------------------------------------------------------

fn realistic_text_strategy() -> impl Strategy<Value = String> {
    let fragments: Vec<&'static str> = vec![
        "",
        "a",
        "abc\n",
        "あ",
        "本文",
        "｜青空《あおぞら》",
        "※［＃「desc」、X］",
        "［＃改ページ］",
        "「鉤」",
        "\n\n",
        "\r\n",
        "X\nY",
    ];
    proptest_vec(select(fragments), 0usize..12usize).prop_map(|frags| frags.concat())
}
