//! Per-document line-start index for `O(log lines)` byte-offset →
//! `Position` conversion.
//!
//! ## Why this exists
//!
//! [`crate::position::byte_offset_to_position`] is `O(byte_offset)`
//! — every call walks the source from byte 0 counting `\n`s and
//! UTF-16 code units. For a single-shot hover that's fine, but
//! `aozora/gaijiSpans` emits two positions per gaiji span, and there
//! can be hundreds of spans in a 40 KB document. Naïvely, that's
//! `N * O(40k) = O(N·doc_size)` per request.
//!
//! [`LineIndex`] precomputes a `Vec<u32>` of line-start byte offsets
//! once per text version. After the index is built, a position
//! conversion is a binary search (`O(log lines)`) plus a UTF-16
//! walk over the *current line slice only* (typically tens of
//! bytes). The whole handler drops from `O(N·doc_size)` to
//! `O(N·log lines + N·avg_line_len)` — effectively linear in the
//! number of spans with a tiny constant.
//!
//! ## Lifecycle
//!
//! Built when [`LineIndex::new`] is called against a fresh source. The
//! `OpenDocument` rebuilds the index on every `did_change` (cheap —
//! a single SIMD `memchr` pass over the full text). Reads are
//! immutable so the index is shared across concurrent LSP
//! handlers without locking.

use tower_lsp::lsp_types::Position;

/// Byte-offset → `Position` accelerator.
///
/// Internally a sorted `Vec<u32>` of line-start byte offsets:
/// `line_starts[N]` is the byte offset of the first character on
/// line N. Line 0 always starts at byte 0.
#[derive(Debug, Clone, Default)]
pub struct LineIndex {
    /// Byte offset where each line begins. `line_starts[0] == 0`.
    /// One entry per line; total length == line count.
    line_starts: Vec<u32>,
}

impl LineIndex {
    /// Build a new index over `source`. `O(source.len())` with SIMD
    /// `memchr` for the newline scan.
    #[must_use]
    pub fn new(source: &str) -> Self {
        let bytes = source.as_bytes();
        // Capacity heuristic: average line length ~64 bytes for
        // prose, undershoots for code. Either way the Vec only
        // grows a handful of times.
        let mut line_starts = Vec::with_capacity(bytes.len() / 64 + 1);
        line_starts.push(0);
        for (idx, &byte) in bytes.iter().enumerate() {
            if byte == b'\n' {
                let start = u32::try_from(idx + 1).unwrap_or(u32::MAX);
                line_starts.push(start);
            }
        }
        Self { line_starts }
    }

    /// Total number of lines in the indexed source. Always `>= 1`.
    #[must_use]
    pub const fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Convert `byte_offset` (clamped to `source.len()`) into an LSP
    /// [`Position`]. `O(log lines)` on the line lookup, `O(line_len)`
    /// on the UTF-16 column walk.
    #[must_use]
    pub fn position(&self, source: &str, byte_offset: usize) -> Position {
        let byte_offset = byte_offset.min(source.len());
        let needle = u32::try_from(byte_offset).unwrap_or(u32::MAX);
        // partition_point: returns the first index where the
        // predicate is false → the line whose start exceeds the
        // needle. The line containing the needle is one before that.
        let line_idx = self
            .line_starts
            .partition_point(|&start| start <= needle)
            .saturating_sub(1);
        let line_start = self.line_starts[line_idx] as usize;
        let line_slice = &source[line_start..byte_offset];
        let col = u32::try_from(line_slice.encode_utf16().count()).unwrap_or(u32::MAX);
        Position::new(u32::try_from(line_idx).unwrap_or(u32::MAX), col)
    }

    /// Convert an LSP [`Position`] back into a byte offset. Returns
    /// `None` if the line number is past EOF; UTF-16 columns past
    /// the end of their line clamp to the line end.
    #[must_use]
    pub fn byte_offset(&self, source: &str, position: Position) -> Option<usize> {
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
    use crate::position::byte_offset_to_position;

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

    /// CRLF line endings: the `\r` is treated as part of the line
    /// (line breaks are split by `\n` only), so a position past the
    /// `\r`'s column resolves to byte position of `\n` (= `line_end`).
    /// Pin so an editor that submits Windows-style line endings
    /// behaves consistently.
    #[test]
    fn crlf_line_ends_keep_cr_inside_the_line() {
        let src = "abc\r\ndef";
        let idx = LineIndex::new(src);
        assert_eq!(idx.line_count(), 2);
        // Byte 3 = '\r', byte 4 = '\n'.
        assert_eq!(idx.position(src, 3), Position::new(0, 3));
        assert_eq!(idx.position(src, 4), Position::new(0, 4));
        // Round-trip: position (0, 4) should resolve back to byte 4.
        assert_eq!(idx.byte_offset(src, Position::new(0, 4)), Some(4));
    }

    /// An `\n`-only document yields two lines: the empty line before
    /// the newline and the empty line after. Pin so a future
    /// "trim trailing newline" change doesn't quietly drop a line.
    #[test]
    fn lone_newline_yields_two_lines() {
        let src = "\n";
        let idx = LineIndex::new(src);
        assert_eq!(idx.line_count(), 2);
        // Position (0, 0) → byte 0; (1, 0) → byte 1 (past the \n).
        assert_eq!(idx.byte_offset(src, Position::new(0, 0)), Some(0));
        assert_eq!(idx.byte_offset(src, Position::new(1, 0)), Some(1));
    }

    /// Empty source: one line, position (0, 0) round-trips, anything
    /// past returns None or clamps to EOF on the position side.
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
