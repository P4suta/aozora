//! Maps source byte offsets to UTF-16 positions through [`LineIndex`].

use tower_lsp::lsp_types::Position;

const CAPACITY_BYTES_PER_LINE: usize = 64;

#[derive(Debug, Clone, Default)]
pub(super) struct LineIndex {
    line_starts: Vec<u32>,
}

impl LineIndex {
    #[must_use]
    pub(super) fn new(source: &str) -> Self {
        let bytes = source.as_bytes();
        let mut line_starts = Vec::with_capacity(bytes.len() / CAPACITY_BYTES_PER_LINE + 1);
        line_starts.push(0);
        for (idx, &byte) in bytes.iter().enumerate() {
            if byte == b'\n' {
                let start = u32::try_from(idx + 1).unwrap_or(u32::MAX);
                line_starts.push(start);
            }
        }
        Self { line_starts }
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Clamps `byte_offset` to the source length before converting it to a position.
    #[must_use]
    pub(super) fn position(&self, source: &str, byte_offset: usize) -> Position {
        let byte_offset = byte_offset.min(source.len());
        let needle = u32::try_from(byte_offset).unwrap_or(u32::MAX);
        let line_idx = self
            .line_starts
            .partition_point(|&start| start <= needle)
            .saturating_sub(1);
        let line_start = self.line_starts[line_idx] as usize;
        let line_slice = &source[line_start..byte_offset];
        let col = u32::try_from(line_slice.encode_utf16().count()).unwrap_or(u32::MAX);
        Position::new(u32::try_from(line_idx).unwrap_or(u32::MAX), col)
    }

    /// Returns `None` for missing lines and clamps columns to their line's end.
    #[must_use]
    pub(super) fn byte_offset(&self, source: &str, position: Position) -> Option<usize> {
        let line_idx = position.line as usize;
        if line_idx >= self.line_starts.len() {
            return None;
        }
        let line_start = self.line_starts[line_idx] as usize;
        let line_end = self
            .line_starts
            .get(line_idx + 1)
            .map_or(source.len(), |next| (*next as usize).saturating_sub(1));
        let line_slice = &source[line_start..line_end];
        let mut utf16_cursor: u32 = 0;
        for (byte_i, ch) in line_slice.char_indices() {
            if utf16_cursor >= position.character {
                return Some(line_start + byte_i);
            }
            utf16_cursor = utf16_cursor.saturating_add(u32::try_from(ch.len_utf16()).unwrap_or(2));
        }
        Some(line_end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lsp::position::byte_offset_to_position;

    fn matches_reference(source: &str, byte_offset: usize) -> bool {
        let idx = LineIndex::new(source);
        let via_index = idx.position(source, byte_offset);
        let via_scan = byte_offset_to_position(source, byte_offset);
        via_index == via_scan
    }

    #[test]
    fn empty_source_has_one_line() {
        assert_eq!(LineIndex::new("").line_count(), 1);
    }

    #[test]
    fn three_lines_yield_three_starts() {
        let idx = LineIndex::new("a\nb\nc");
        assert_eq!(idx.line_count(), 3);
    }

    #[test]
    fn position_at_origin_matches_reference() {
        assert!(matches_reference("hello", 0));
    }

    #[test]
    fn position_after_newlines_matches_reference() {
        let src = "one\ntwo\nthree";
        for offset in 0..=src.len() {
            if !src.is_char_boundary(offset) {
                continue;
            }
            assert!(matches_reference(src, offset), "offset {offset}");
        }
    }

    #[test]
    fn utf8_multibyte_columns_match_reference() {
        let src = "あいう\nえおか";
        for offset in 0..=src.len() {
            if !src.is_char_boundary(offset) {
                continue;
            }
            assert!(matches_reference(src, offset), "offset {offset}");
        }
    }

    #[test]
    fn surrogate_pair_columns_match_reference() {
        let src = "a😀b\nc😀d";
        for offset in 0..=src.len() {
            if !src.is_char_boundary(offset) {
                continue;
            }
            assert!(matches_reference(src, offset), "offset {offset}");
        }
    }

    #[test]
    fn round_trip_via_index_is_identity() {
        let src = "abc\nあいう\ndef";
        let idx = LineIndex::new(src);
        for byte in 0..=src.len() {
            if !src.is_char_boundary(byte) {
                continue;
            }
            let pos = idx.position(src, byte);
            let round = idx.byte_offset(src, pos).expect("position is in range");
            assert_eq!(
                round, byte,
                "byte {byte} round-tripped to {round} via {pos:?}"
            );
        }
    }

    #[test]
    fn overshoot_clamps_to_eof() {
        let idx = LineIndex::new("hi");
        assert_eq!(idx.position("hi", 99), Position::new(0, 2));
    }

    #[test]
    fn position_past_eof_returns_none() {
        let idx = LineIndex::new("one\ntwo");
        assert_eq!(idx.byte_offset("one\ntwo", Position::new(5, 0)), None);
    }

    #[test]
    fn crlf_line_ends_keep_cr_inside_the_line() {
        let src = "abc\r\ndef";
        let idx = LineIndex::new(src);
        assert_eq!(idx.line_count(), 2);
        assert_eq!(idx.position(src, 3), Position::new(0, 3));
        assert_eq!(idx.position(src, 4), Position::new(0, 4));
        assert_eq!(idx.byte_offset(src, Position::new(0, 4)), Some(4));
    }

    #[test]
    fn lone_newline_yields_two_lines() {
        let src = "\n";
        let idx = LineIndex::new(src);
        assert_eq!(idx.line_count(), 2);
        assert_eq!(idx.byte_offset(src, Position::new(0, 0)), Some(0));
        assert_eq!(idx.byte_offset(src, Position::new(1, 0)), Some(1));
    }

    #[test]
    fn empty_source_round_trips_origin_position() {
        let src = "";
        let idx = LineIndex::new(src);
        assert_eq!(idx.line_count(), 1);
        assert_eq!(idx.position(src, 0), Position::new(0, 0));
        assert_eq!(idx.byte_offset(src, Position::new(0, 0)), Some(0));
        assert_eq!(idx.byte_offset(src, Position::new(0, 99)), Some(0));
    }
}
