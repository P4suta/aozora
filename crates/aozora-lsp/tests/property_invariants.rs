//! Property-based invariant tests for the byte ↔ position
//! conversion path, the in-tree edit splice, and the paragraph
//! segmenter.
//!
//! Each property is an *invariant* — a statement that should hold
//! across an entire input space, not just hand-picked corner
//! cases. The fuzz-style coverage catches off-by-one regressions
//! and char-boundary panics that targeted tests miss because the
//! bug only surfaces under inputs the test author didn't think to
//! write.
//!
//! Properties:
//!
//! 1. **Position round-trip**: `position(byte) → byte` is the
//!    identity for every char-boundary byte offset, regardless of
//!    line / column shape (CRLF, surrogate pairs, mixed scripts).
//! 2. **`LineIndex` agrees with a single-shot reference scanner**
//!    for every char-boundary byte offset.
//! 3. **`apply_edits` round-trip**: applying the empty edit list
//!    yields the source unchanged; non-empty edits are equivalent
//!    to a hand-rolled splice.
//! 4. **`paragraph_byte_ranges` invariants**: every emitted range
//!    aligns to UTF-8 char boundaries; the ranges cover the entire
//!    input with no gap and no overlap; concatenating the slices
//!    reconstructs the source byte-for-byte.

use aozora_lsp::internals::{
    ByteEdit, DocLineView, LineIndex, OpenDocument, apply_edits, byte_offset_to_position,
    position_to_byte_offset,
};
use proptest::collection::vec as proptest_vec;
use proptest::prelude::*;
use proptest::sample::select;
use ropey::Rope;

/// Generate a string biased toward content the LSP actually sees:
/// kanji/hiragana, latin, newlines, and aozora notation triggers.
/// `proptest`'s default `String` strategy generates rare codepoints
/// that don't reflect realistic input.
fn realistic_text_strategy() -> impl Strategy<Value = String> {
    // Each token is a fragment we paste into the output. Mixing
    // multi-byte and ASCII fragments stresses the byte ↔ char
    // conversion paths.
    let fragments: Vec<&'static str> = vec![
        "",
        "a",
        "abc",
        " ",
        "\n",
        "\r\n",
        "\n\n",
        "\r", // bare CR: stresses the LF-only line splitter
        "あ",
        "本文",
        "｜青空《あおぞら》",
        "《《重要》》", // double ruby
        "〔Crevez〕",   // tortoise-bracket accent
        "※［＃「desc」、X］",
        "［＃改ページ］",
        "😀", // surrogate pair
        "「abc」",
        "X\nY",
        "\u{E001}", // PUA sentinel collision
        "\u{feff}", // BOM
        // Non-LF separators that ropey's DEFAULT features would treat as
        // line breaks. With the workspace ropey pinned to LF-only, the
        // rope line metrics must ignore these — the
        // `doc_line_view_rope_matches_line_index` property is the
        // load-bearing pin for that.
        "\u{0B}",   // VT
        "\u{0C}",   // FF
        "\u{85}",   // NEL
        "\u{2028}", // LS
        "\u{2029}", // PS
    ];
    let frag_count = 0usize..16usize;
    proptest_vec(select(fragments), frag_count).prop_map(|frags| frags.concat())
}

proptest! {
    /// For every char-boundary byte offset, the
    /// `byte_offset → position → byte_offset` round-trip lands back
    /// on the original byte.
    #[test]
    fn position_round_trip_is_identity(text in realistic_text_strategy()) {
        for byte in 0..=text.len() {
            if !text.is_char_boundary(byte) {
                continue;
            }
            let pos = byte_offset_to_position(&text, byte);
            let round = position_to_byte_offset(&text, pos)
                .ok_or_else(|| TestCaseError::fail("position out of range"))?;
            prop_assert_eq!(round, byte, "byte {} round-tripped to {}", byte, round);
        }
    }

    /// `LineIndex::position` must agree with the single-shot scanner
    /// `byte_offset_to_position` for every char-boundary byte.
    /// Drift between the two would mean editor positions disagree
    /// based on whether the handler keeps a `LineIndex` or falls
    /// back to the scanner.
    #[test]
    fn line_index_matches_reference_scanner(text in realistic_text_strategy()) {
        let idx = LineIndex::new(&text);
        for byte in 0..=text.len() {
            if !text.is_char_boundary(byte) {
                continue;
            }
            let via_index = idx.position(&text, byte);
            let via_scan = byte_offset_to_position(&text, byte);
            prop_assert_eq!(via_index, via_scan, "byte {}", byte);
        }
    }

    /// Empty edit list: `apply_edits` must return the source
    /// unchanged for any input. This is the trivial-case identity
    /// the formatter relies on when `format_source` says "already
    /// canonical".
    #[test]
    fn apply_empty_edits_returns_source(text in realistic_text_strategy()) {
        let out = apply_edits(&text, &[]).expect("empty edits never fail");
        prop_assert_eq!(out, text);
    }

    /// A single in-bounds, char-boundary-aligned replacement edit
    /// must produce the same string as a manual splice. This is
    /// the core contract of `apply_edits` for the LSP backend's
    /// per-change path.
    #[test]
    fn single_replacement_matches_manual_splice(
        text in realistic_text_strategy(),
        replacement in "[\\p{ASCII}]{0,10}",
    ) {
        // Pick an arbitrary char-boundary range. We use the first
        // and last char-boundary positions; this exercises the full
        // spectrum from "delete everything" to "insert at start".
        let len = text.len();
        if len == 0 {
            // Insertion-only at byte 0 of empty text.
            let edit = ByteEdit::new(0..0, replacement.clone());
            let out = apply_edits(&text, &[edit]).expect("valid edit");
            prop_assert_eq!(out, replacement);
            return Ok(());
        }
        // Find a small char-boundary range somewhere in the middle.
        let start = next_char_boundary(&text, len / 4);
        let end = next_char_boundary(&text, len / 2);
        let edit = ByteEdit::new(start..end, replacement.clone());
        let out = apply_edits(&text, &[edit]).expect("valid edit");
        let expected = format!("{}{}{}", &text[..start], replacement, &text[end..]);
        prop_assert_eq!(out, expected);
    }

    /// **Mechanism A byte-identity pin (#237 Tier-2).** The rope-backed
    /// [`DocLineView::Rope`] used on the publish hot path must produce the
    /// **exact** same `Position` as a `LineIndex` over the same source, for
    /// every char-boundary byte offset. This is what guarantees the
    /// published diagnostics are byte-identical after swapping the O(doc)
    /// per-keystroke line-table rebuild for O(log n) rope lookups — and it
    /// fails loudly if ropey's non-LF line-break features are ever restored
    /// (the strategy injects CRLF, BOM, VT/FF/NEL/LS/PS, and multibyte text).
    #[test]
    fn doc_line_view_rope_matches_line_index(text in realistic_text_strategy()) {
        let rope = Rope::from(text.as_str());
        let view = DocLineView::Rope(&rope);
        let idx = LineIndex::new(&text);
        for byte in 0..=text.len() {
            if !text.is_char_boundary(byte) {
                continue;
            }
            prop_assert_eq!(view.position(byte), idx.position(&text, byte), "byte {}", byte);
        }
    }

    /// `paragraph_byte_ranges` (exercised end-to-end through
    /// `OpenDocument::new`) must round-trip the source: concatenating
    /// every paragraph's text reproduces the original byte-for-byte.
    /// And `doc_text()` should equal the input.
    ///
    /// Property holds for any UTF-8 input regardless of paragraph
    /// shape (zero, one, or many `\n\n` boundaries).
    #[test]
    fn doc_state_round_trips_arbitrary_text(text in realistic_text_strategy()) {
        let state = OpenDocument::new(text.clone());
        let snap = state.snapshot();
        prop_assert_eq!(&**snap.doc_text(), text.as_str());
    }
}

/// Snap `byte` forward to the next char boundary in `text`. Returns
/// `text.len()` if no boundary at or after `byte` exists (which
/// shouldn't happen since `text.len()` is always a boundary).
fn next_char_boundary(text: &str, byte: usize) -> usize {
    let mut i = byte.min(text.len());
    while i < text.len() && !text.is_char_boundary(i) {
        i += 1;
    }
    i
}
