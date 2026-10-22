use tower_lsp::lsp_types::{LinkedEditingRanges, Position, Range};

use crate::lsp::line_index::LineIndex;

const PAIRS: &[(char, char)] = &[('［', '］'), ('《', '》'), ('「', '」'), ('〔', '〕')];

const SCAN_WINDOW: usize = 1024;

#[must_use]
pub(super) fn linked_editing_at(
    source: &str,
    line_index: &LineIndex,
    position: Position,
) -> Option<LinkedEditingRanges> {
    let cursor = line_index.byte_offset(source, position)?;

    let candidates = [
        char_at_offset(source, cursor),
        char_before_offset(source, cursor),
    ];

    for cand in candidates.into_iter().flatten() {
        if let Some(link) = try_link(source, line_index, cand) {
            return Some(link);
        }
    }
    None
}

fn char_at_offset(source: &str, offset: usize) -> Option<(usize, char, usize)> {
    if offset >= source.len() || !source.is_char_boundary(offset) {
        return None;
    }
    let ch = source[offset..].chars().next()?;
    Some((offset, ch, offset + ch.len_utf8()))
}

fn char_before_offset(source: &str, offset: usize) -> Option<(usize, char, usize)> {
    let head = source.get(..offset)?;
    let (start, ch) = head.char_indices().next_back()?;
    Some((start, ch, offset))
}

fn try_link(
    source: &str,
    line_index: &LineIndex,
    (start, ch, end): (usize, char, usize),
) -> Option<LinkedEditingRanges> {
    let (partner, search_forward) = PAIRS.iter().find_map(|&(o, c)| {
        if ch == o {
            Some((c, true))
        } else if ch == c {
            Some((o, false))
        } else {
            None
        }
    })?;

    let partner_span = if search_forward {
        find_partner_forward(source, end, partner)?
    } else {
        find_partner_backward(source, start, partner)?
    };

    let here_range = Range::new(
        line_index.position(source, start),
        line_index.position(source, end),
    );
    let partner_range = Range::new(
        line_index.position(source, partner_span.0),
        line_index.position(source, partner_span.1),
    );
    let (open, close) = if search_forward {
        (here_range, partner_range)
    } else {
        (partner_range, here_range)
    };
    Some(LinkedEditingRanges {
        ranges: vec![open, close],
        word_pattern: None,
    })
}

fn find_partner_forward(source: &str, start: usize, target: char) -> Option<(usize, usize)> {
    for (rel, ch) in source
        .get(start..)?
        .char_indices()
        .take_while(|(rel, _)| *rel < SCAN_WINDOW)
    {
        if ch == '\n' {
            return None;
        }
        if ch == target {
            let idx = start + rel;
            return Some((idx, idx + ch.len_utf8()));
        }
    }
    None
}

fn find_partner_backward(source: &str, end: usize, target: char) -> Option<(usize, usize)> {
    let floor =
        (end.saturating_sub(SCAN_WINDOW)..=end).find(|&offset| source.is_char_boundary(offset))?;
    let head = source.get(floor..end)?;
    for (rel, ch) in head.char_indices().rev() {
        if ch == '\n' {
            return None;
        }
        if ch == target {
            let abs = floor + rel;
            return Some((abs, abs + ch.len_utf8()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(source: &str, byte_offset: usize) -> Position {
        LineIndex::new(source).position(source, byte_offset)
    }

    #[test]
    fn ruby_open_links_to_close() {
        let src = "｜青空《あおぞら》";
        let open_byte = src.find('《').unwrap();
        let result =
            linked_editing_at(src, &LineIndex::new(src), pos(src, open_byte)).expect("link");
        assert_eq!(result.ranges.len(), 2);
        let close_byte = src.find('》').unwrap();
        assert_eq!(result.ranges[1].start, pos(src, close_byte));
    }

    #[test]
    fn ruby_close_links_back_to_open() {
        let src = "｜青空《あおぞら》";
        let close_byte = src.find('》').unwrap();
        let result =
            linked_editing_at(src, &LineIndex::new(src), pos(src, close_byte)).expect("link");
        let open_byte = src.find('《').unwrap();
        assert_eq!(result.ranges[0].start, pos(src, open_byte));
        assert_eq!(result.ranges[1].start, pos(src, close_byte));
    }

    #[test]
    fn slug_brackets_link() {
        let src = "前置き［＃改ページ］後";
        let open_byte = src.find('［').unwrap();
        let result =
            linked_editing_at(src, &LineIndex::new(src), pos(src, open_byte)).expect("link");
        let close_byte = src.find('］').unwrap();
        assert_eq!(result.ranges[1].start, pos(src, close_byte));
    }

    #[test]
    fn quote_brackets_link() {
        let src = "「ほら」と言った";
        let open_byte = src.find('「').unwrap();
        let result =
            linked_editing_at(src, &LineIndex::new(src), pos(src, open_byte)).expect("link");
        assert_eq!(result.ranges[1].start, pos(src, src.find('」').unwrap()));
    }

    #[test]
    fn cursor_just_after_opener_also_fires() {
        let src = "｜青空《あおぞら》";
        let after_open = src.find('《').unwrap() + '《'.len_utf8();
        let result =
            linked_editing_at(src, &LineIndex::new(src), pos(src, after_open)).expect("link");
        assert_eq!(result.ranges[1].start, pos(src, src.find('》').unwrap()));
    }

    #[test]
    fn no_link_in_plain_text() {
        let src = "ただの文章";
        assert!(linked_editing_at(src, &LineIndex::new(src), pos(src, 3)).is_none());
    }

    #[test]
    fn scan_does_not_cross_newlines() {
        let src = "前《ほげ\nふが》後";
        let open_byte = src.find('《').unwrap();
        assert!(linked_editing_at(src, &LineIndex::new(src), pos(src, open_byte)).is_none());
    }

    #[test]
    fn ascii_brackets_are_intentionally_unsupported() {
        let src = "[hello]";
        let open_byte = src.find('[').unwrap();
        assert!(linked_editing_at(src, &LineIndex::new(src), pos(src, open_byte)).is_none());
    }

    #[test]
    fn scan_caps_at_window() {
        let filler = "x".repeat(SCAN_WINDOW + 100);
        let src = format!("《{filler}》");
        let open_byte = src.find('《').unwrap();
        assert!(linked_editing_at(&src, &LineIndex::new(&src), pos(&src, open_byte)).is_none());
    }

    #[test]
    fn forward_scan_does_not_panic_on_mid_codepoint_window_cap() {
        let filler = "あ".repeat(SCAN_WINDOW);
        let src = format!("《{filler}》");
        let open_byte = src.find('《').unwrap();
        let result = linked_editing_at(&src, &LineIndex::new(&src), pos(&src, open_byte));
        assert!(result.is_none(), "expected None, got {result:?}");
    }

    #[test]
    fn backward_scan_does_not_panic_on_mid_codepoint_floor() {
        let filler = "あ".repeat(SCAN_WINDOW);
        let src = format!("《{filler}》");
        let close_byte = src.rfind('》').unwrap();
        let result = linked_editing_at(&src, &LineIndex::new(&src), pos(&src, close_byte));
        assert!(result.is_none(), "expected None, got {result:?}");
    }

    #[test]
    fn forward_scan_finds_partner_through_multibyte_filler() {
        let filler = "あ".repeat(50);
        let src = format!("《{filler}》");
        let open_byte = src.find('《').unwrap();
        let result =
            linked_editing_at(&src, &LineIndex::new(&src), pos(&src, open_byte)).expect("link");
        let close_byte = src.rfind('》').unwrap();
        assert_eq!(result.ranges[1].start, pos(&src, close_byte));
    }

    #[test]
    fn char_at_offset_rejects_mid_codepoint_offset() {
        assert!(char_at_offset("あい", 1).is_none());
    }

    #[test]
    fn forward_scan_excludes_char_exactly_at_window_edge() {
        let src = format!("{}》", "x".repeat(SCAN_WINDOW));
        assert_eq!(find_partner_forward(&src, 0, '》'), None);
    }

    #[test]
    fn forward_scan_measures_distance_from_start_not_sum() {
        let src = format!("{}》", "x".repeat(700));
        assert_eq!(find_partner_forward(&src, 700, '》'), Some((700, 703)));
    }

    #[test]
    fn forward_scan_partner_end_is_start_plus_char_len() {
        assert_eq!(find_partner_forward("xxx》", 0, '》'), Some((3, 6)));
    }

    #[test]
    fn backward_scan_snaps_floor_up_to_boundary() {
        let src = format!("xxx《{}", "x".repeat(1022));
        assert_eq!(src.len(), 1028);
        assert_eq!(find_partner_backward(&src, src.len(), '《'), None);
    }

    #[test]
    fn backward_scan_partner_end_is_abs_plus_char_len() {
        assert_eq!(find_partner_backward("yyy《xx", 8, '《'), Some((3, 6)));
    }
}
