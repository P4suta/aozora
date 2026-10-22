use ropey::Rope;
use tower_lsp::lsp_types::Position;

#[cfg(test)]
use crate::lsp::line_index::LineIndex;

#[derive(Debug)]
pub(super) enum DocLineView<'a> {
    Rope(&'a Rope),
    #[cfg(test)]
    Str {
        source: &'a str,
        index: LineIndex,
    },
}

#[cfg(test)]
impl<'a> DocLineView<'a> {
    #[must_use]
    pub(crate) fn from_source(source: &'a str) -> Self {
        Self::Str {
            source,
            index: LineIndex::new(source),
        }
    }
}

impl DocLineView<'_> {
    #[must_use]
    pub(super) fn position(&self, byte_offset: usize) -> Position {
        match self {
            #[cfg(test)]
            Self::Str { source, index } => index.position(source, byte_offset),
            Self::Rope(rope) => rope_position(rope, byte_offset),
        }
    }
}

fn rope_position(raw: &Rope, byte_offset: usize) -> Position {
    let byte_offset = byte_offset.min(raw.len_bytes());
    let line = raw.byte_to_line(byte_offset);
    let line_start = raw.line_to_byte(line);
    let col: usize = raw
        .byte_slice(line_start..byte_offset)
        .chars()
        .map(char::len_utf16)
        .sum();
    Position::new(
        u32::try_from(line).unwrap_or(u32::MAX),
        u32::try_from(col).unwrap_or(u32::MAX),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ropey_counts_only_lf() {
        let r = Rope::from("a\rb\u{0B}c\u{0C}d\u{85}e\u{2028}f\u{2029}g");
        assert_eq!(r.len_lines(), 1);
    }

    #[test]
    fn rope_arm_matches_line_index_on_crlf() {
        let src = "abc\r\ndef";
        let rope = Rope::from(src);
        let view = DocLineView::Rope(&rope);
        let idx = LineIndex::new(src);
        for byte in 0..=src.len() {
            if !src.is_char_boundary(byte) {
                continue;
            }
            assert_eq!(view.position(byte), idx.position(src, byte), "byte {byte}");
        }
    }

    #[test]
    fn str_arm_matches_line_index() {
        let src = "あ\nい\u{2028}う";
        let view = DocLineView::from_source(src);
        let idx = LineIndex::new(src);
        for byte in 0..=src.len() {
            if !src.is_char_boundary(byte) {
                continue;
            }
            assert_eq!(view.position(byte), idx.position(src, byte), "byte {byte}");
        }
    }

    #[test]
    fn rope_arm_overshoot_clamps_to_eof() {
        let rope = Rope::from("hi");
        assert_eq!(DocLineView::Rope(&rope).position(99), Position::new(0, 2));
    }
}
