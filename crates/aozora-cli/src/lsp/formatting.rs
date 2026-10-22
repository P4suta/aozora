use tower_lsp::lsp_types::{Position, Range, TextEdit};

use aozora::fmt::format_source;

use crate::lsp::position::byte_offset_to_position;

#[must_use]
pub(super) fn format_edits(source: &str) -> Vec<TextEdit> {
    let formatted = format_source(source);
    if formatted == source {
        return Vec::new();
    }
    let end = byte_offset_to_position(source, source.len());
    vec![TextEdit {
        range: Range::new(Position::new(0, 0), end),
        new_text: formatted,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_input_produces_no_edits() {
        assert_eq!(format_edits("日本《にほん》"), Vec::<TextEdit>::new());
    }

    #[test]
    fn non_canonical_ruby_produces_one_replace_edit() {
        let src = "｜日本《にほん》";
        let edits = format_edits(src);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start, Position::new(0, 0));
        assert!(
            edits[0].new_text.starts_with("日本《にほん》") && !edits[0].new_text.contains('｜')
        );
    }

    #[test]
    fn empty_source_produces_no_edits() {
        assert_eq!(format_edits(""), Vec::<TextEdit>::new());
    }
}
