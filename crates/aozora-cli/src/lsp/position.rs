use tower_lsp::lsp_types::Position;

#[must_use]
pub(super) fn byte_offset_to_position(source: &str, byte_offset: usize) -> Position {
    let byte_offset = byte_offset.min(source.len());
    let prefix = &source[..byte_offset];
    let line = u32::try_from(prefix.matches('\n').count()).unwrap_or(u32::MAX);
    let line_start = prefix.rfind('\n').map_or(0, |idx| idx + 1);
    let col =
        u32::try_from(source[line_start..byte_offset].encode_utf16().count()).unwrap_or(u32::MAX);
    Position::new(line, col)
}

#[must_use]
pub(super) fn position_to_byte_offset(source: &str, position: Position) -> Option<usize> {
    let line_start = if position.line == 0 {
        0
    } else {
        let target_index = (position.line as usize).checked_sub(1)?;
        let (newline_pos, _) = source.match_indices('\n').nth(target_index)?;
        newline_pos + 1
    };
    let line_end = source[line_start..]
        .find('\n')
        .map_or(source.len(), |p| line_start + p);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_byte_maps_to_origin() {
        assert_eq!(byte_offset_to_position("hello", 0), Position::new(0, 0));
    }

    #[test]
    fn last_byte_maps_to_end_of_line() {
        assert_eq!(byte_offset_to_position("hello", 5), Position::new(0, 5));
    }

    #[test]
    fn overshoot_clamps_to_eof() {
        assert_eq!(byte_offset_to_position("hi", 99), Position::new(0, 2));
    }

    #[test]
    fn newline_advances_line() {
        let src = "one\ntwo";
        assert_eq!(byte_offset_to_position(src, 4), Position::new(1, 0));
        assert_eq!(byte_offset_to_position(src, 7), Position::new(1, 3));
    }

    #[test]
    fn utf8_multibyte_uses_utf16_column() {
        let src = "あいう";
        assert_eq!(byte_offset_to_position(src, 3), Position::new(0, 1));
        assert_eq!(byte_offset_to_position(src, 6), Position::new(0, 2));
        assert_eq!(byte_offset_to_position(src, 9), Position::new(0, 3));
    }

    #[test]
    fn surrogate_pair_counts_two_utf16_units() {
        let src = "a😀b";
        assert_eq!(byte_offset_to_position(src, 5), Position::new(0, 3));
        assert_eq!(byte_offset_to_position(src, 6), Position::new(0, 4));
    }

    #[test]
    fn round_trip_origin() {
        let src = "hello";
        assert_eq!(position_to_byte_offset(src, Position::new(0, 0)), Some(0));
    }

    #[test]
    fn round_trip_newline_line_2() {
        let src = "one\ntwo\nthree";
        assert_eq!(position_to_byte_offset(src, Position::new(2, 0)), Some(8));
        assert_eq!(position_to_byte_offset(src, Position::new(2, 5)), Some(13));
    }

    #[test]
    fn position_past_end_of_buffer_returns_none() {
        let src = "one";
        assert_eq!(position_to_byte_offset(src, Position::new(5, 0)), None);
    }

    #[test]
    fn utf16_column_past_line_end_clamps_to_line_end() {
        let src = "abc\ndef";
        assert_eq!(position_to_byte_offset(src, Position::new(0, 99)), Some(3));
    }

    #[test]
    fn byte_to_position_and_back_is_identity() {
        let src = "abc\nあいう\ndef";
        for byte in 0..=src.len() {
            if !src.is_char_boundary(byte) {
                continue;
            }
            let pos = byte_offset_to_position(src, byte);
            let round = position_to_byte_offset(src, pos).expect("round-trip");
            assert_eq!(
                round, byte,
                "byte {byte} round-tripped to {round} via {pos:?}"
            );
        }
    }
}
