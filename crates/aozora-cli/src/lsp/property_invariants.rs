//! Compares source positions, line views, and document edits with reference operations.

use proptest::collection::vec as proptest_vec;
use proptest::prelude::*;
use proptest::sample::select;
use ropey::Rope;

use super::doc_line_view::DocLineView;
use super::line_index::LineIndex;
use super::position::{byte_offset_to_position, position_to_byte_offset};
use super::state::OpenDocument;
use super::text_edit::ByteEdit;

const TEXT_FRAGMENT_COUNT_LIMIT: usize = 16;
const EDIT_START_LENGTH_DIVISOR: usize = 4;

fn realistic_text_strategy() -> impl Strategy<Value = String> {
    let fragments: Vec<&'static str> = vec![
        "",
        "a",
        "abc",
        " ",
        "\n",
        "\r\n",
        "\n\n",
        "\r",
        "あ",
        "本文",
        "｜青空《あおぞら》",
        "《《重要》》",
        "〔Crevez〕",
        "※［＃「desc」、X］",
        "［＃改ページ］",
        "😀",
        "「abc」",
        "X\nY",
        "\u{E001}",
        "\u{feff}",
        "\u{0B}",
        "\u{0C}",
        "\u{85}",
        "\u{2028}",
        "\u{2029}",
    ];
    let frag_count = 0usize..TEXT_FRAGMENT_COUNT_LIMIT;
    proptest_vec(select(fragments), frag_count).prop_map(|frags| frags.concat())
}

proptest! {
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

    #[test]
    fn apply_empty_edits_returns_source(text in realistic_text_strategy()) {
        let mut document = aozora::parse(text.clone()).expect("generated source fits");
        document.edit([]).expect("empty edits never fail");
        prop_assert_eq!(document.source(), text);
    }

    #[test]
    fn single_replacement_matches_manual_splice(
        text in realistic_text_strategy(),
        replacement in "[\\p{ASCII}]{0,10}",
    ) {
        let len = text.len();
        if len == 0 {
            let edit = ByteEdit::new(0..0, replacement.clone());
            let mut document = aozora::parse(text).expect("generated source fits");
            document
                .edit([aozora::TextEdit::new(edit.range, edit.new_text)])
                .expect("valid edit");
            prop_assert_eq!(document.source(), replacement);
            return Ok(());
        }
        let start = next_char_boundary(&text, len / EDIT_START_LENGTH_DIVISOR);
        let end = next_char_boundary(&text, len / 2);
        let edit = ByteEdit::new(start..end, replacement.clone());
        let mut document = aozora::parse(text.clone()).expect("generated source fits");
        document
            .edit([aozora::TextEdit::new(edit.range, edit.new_text)])
            .expect("valid edit");
        let expected = format!("{}{}{}", &text[..start], replacement, &text[end..]);
        prop_assert_eq!(document.source(), expected);
    }

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

    #[test]
    fn doc_state_round_trips_arbitrary_text(text in realistic_text_strategy()) {
        let state = OpenDocument::new(text.clone());
        let snap = state.snapshot();
        prop_assert_eq!(snap.doc_text(), text.as_str());
    }
}

fn next_char_boundary(text: &str, byte: usize) -> usize {
    let mut i = byte.min(text.len());
    while i < text.len() && !text.is_char_boundary(i) {
        i += 1;
    }
    i
}
