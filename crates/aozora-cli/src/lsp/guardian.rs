//! Exercises language-server providers with adversarial input, edits, and concurrent reads.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use aozora::fmt::format_source;
use proptest::collection::vec as proptest_vec;
use proptest::prelude::*;
use proptest::sample::select;
use tower_lsp::lsp_types::{Position, Range, Url};

use crate::i18n::LanguageIdentifier;

use super::code_actions::wrap_selection_actions;
use super::completion::completion_at;
use super::diagnostics::diagnostics_for_source;
use super::document_symbol::document_symbols;
use super::folding_range::folding_ranges;
use super::formatting::format_edits;
use super::half_width_emmet::emmet_completions;
use super::hover::hover_at;
use super::line_index::LineIndex;
use super::linked_editing::linked_editing_at;
use super::on_type_formatting::format_on_type;
use super::position::{byte_offset_to_position, position_to_byte_offset};
use super::state::OpenDocument;
use super::structured_snippets::snippet_completions;
use super::text_edit::ByteEdit;

const KANA_RUN_CHAR_COUNT: usize = 100;
const OVERSIZED_PARAGRAPH_CHAR_COUNT: usize = 30_000;
const ASCII_LINE_BYTE_COUNT: usize = 5_000;
const PARAGRAPH_REPEAT_COUNT: usize = 50;
const PAST_LINE_END_COLUMN: u32 = 99;
const PAST_SOURCE_POSITION_COMPONENT: u32 = 50;
const FORMAT_FRAGMENT_COUNT_LIMIT: usize = 12;

fn en() -> LanguageIdentifier {
    "en".parse().expect("en parses")
}

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
    "［＃改ページ",
    "本文 ］",
    "《reading》",
    "｜",
    "※",
    "a😀b\nc😀d",
    "abc\u{E001}def",
];

fn adversarial_corpus() -> Vec<String> {
    let mut out: Vec<String> = PANIC_CORPUS.iter().map(|&s| s.to_owned()).collect();
    out.push("あ".repeat(KANA_RUN_CHAR_COUNT));
    out.push("あ".repeat(OVERSIZED_PARAGRAPH_CHAR_COUNT));
    out.push("x".repeat(ASCII_LINE_BYTE_COUNT));
    out.push("x\n\n".repeat(PARAGRAPH_REPEAT_COUNT));
    out
}

fn position_corpus() -> Vec<Position> {
    vec![
        Position::new(0, 0),
        Position::new(0, 1),
        Position::new(0, PAST_LINE_END_COLUMN),
        Position::new(1, 0),
        Position::new(
            PAST_SOURCE_POSITION_COMPONENT,
            PAST_SOURCE_POSITION_COMPONENT,
        ),
        Position::new(u32::MAX, u32::MAX),
        Position::new(u32::MAX, 0),
        Position::new(0, u32::MAX),
    ]
}

#[test]
fn hover_never_panics_on_corpus() {
    for src in adversarial_corpus() {
        let document = aozora::parse(src.as_str()).expect("test source is within parser limit");
        let snapshot = document.snapshot();
        for pos in position_corpus() {
            drop(hover_at(&snapshot, pos, &en()));
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
        drop(document_symbols(&src, &en()));
    }
}

#[test]
fn position_byte_round_trip_never_panics_on_boundary_offsets() {
    for src in adversarial_corpus() {
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
        Range::new(Position::new(0, 5), Position::new(0, 1)),
    ];
    for src in adversarial_corpus() {
        let idx = LineIndex::new(&src);
        for r in ranges {
            drop(wrap_selection_actions(&src, &idx, &uri, r, &en()));
        }
    }
}

proptest! {
    #[test]
    fn format_is_idempotent_for_realistic_inputs(text in realistic_text_strategy()) {
        let once = format_source(&text);
        let twice = format_source(&once);
        prop_assert_eq!(once, twice);
    }
}

struct Xorshift(u64);
impl Xorshift {
    const NONZERO_SEED: u64 = 0xDEAD_BEEF_CAFE_BABE;
    const INITIAL_LEFT_SHIFT: u32 = 13;
    const RIGHT_SHIFT: u32 = 7;
    const FINAL_LEFT_SHIFT: u32 = 17;

    const fn new(seed: u64) -> Self {
        Self(if seed == 0 { Self::NONZERO_SEED } else { seed })
    }
    const fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << Self::INITIAL_LEFT_SHIFT;
        x ^= x >> Self::RIGHT_SHIFT;
        x ^= x << Self::FINAL_LEFT_SHIFT;
        self.0 = x;
        x
    }
    fn range(&mut self, lo: usize, hi_inclusive: usize) -> usize {
        if hi_inclusive <= lo {
            return lo;
        }
        let span = hi_inclusive - lo + 1;
        let bits = self.next() & u64::from(u32::MAX);
        lo + (usize::try_from(bits).unwrap_or(0) % span)
    }
    const fn flip(&mut self) -> bool {
        self.next() & 1 == 1
    }
}

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
    assert_eq!(state.snapshot().doc_text(), oracle);
}

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
            let ch = (b'A' + ((rng.next() % 26) as u8)) as char;
            ch.to_string()
        };
        let edit = ByteEdit::new(start..end, new_text.clone());
        if state.apply_changes(&[edit]).is_some() {
            oracle.replace_range(start..end, &new_text);
        }
    }
    assert_eq!(state.snapshot().doc_text(), oracle);
}

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
                assert!(snap.version >= last_seen_version);
                last_seen_version = snap.version;
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

#[test]
fn diagnostics_for_source_returns_valid_ranges_for_corpus() {
    for src in adversarial_corpus() {
        let diags = diagnostics_for_source(&src, &en());
        let idx = LineIndex::new(&src);
        for d in diags {
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
    proptest_vec(select(fragments), 0usize..FORMAT_FRAGMENT_COUNT_LIMIT)
        .prop_map(|frags| frags.concat())
}
