use tower_lsp::lsp_types::{FoldingRange, FoldingRangeKind};

#[must_use]
pub(super) fn folding_ranges(source: &str) -> Vec<FoldingRange> {
    let mut out = Vec::new();
    block_ranges(source, &mut out);
    heading_ranges(source, &mut out);
    out.sort_by_key(|r| r.start_line);
    out
}

fn block_ranges(source: &str, out: &mut Vec<FoldingRange>) {
    const OPEN: &str = "［＃ここから";
    const CLOSE: &str = "終わり］";
    let mut stack: Vec<u32> = Vec::new();
    for (line_idx, line) in source.lines().enumerate() {
        let line_idx = u32::try_from(line_idx).unwrap_or(u32::MAX);
        for _ in line.match_indices(OPEN) {
            stack.push(line_idx);
        }
        for _ in line.match_indices(CLOSE) {
            if let Some(start_line) = stack.pop()
                && line_idx > start_line
            {
                out.push(FoldingRange {
                    start_line,
                    start_character: None,
                    end_line: line_idx,
                    end_character: None,
                    kind: Some(FoldingRangeKind::Region),
                    collapsed_text: None,
                });
            }
        }
    }
}

fn heading_ranges(source: &str, out: &mut Vec<FoldingRange>) {
    const HEADING_MARKER: &str = "見出し］";
    let last_line = u32::try_from(source.lines().count().saturating_sub(1)).unwrap_or(0);
    let mut anchors: Vec<u32> = Vec::new();
    for (line_idx, line) in source.lines().enumerate() {
        if line.contains(HEADING_MARKER) {
            anchors.push(u32::try_from(line_idx).unwrap_or(u32::MAX));
        }
    }
    for window in anchors.windows(2) {
        let start = window[0];
        let end = window[1].saturating_sub(1);
        if end > start {
            out.push(FoldingRange {
                start_line: start,
                start_character: None,
                end_line: end,
                end_character: None,
                kind: Some(FoldingRangeKind::Region),
                collapsed_text: None,
            });
        }
    }
    if let Some(&last_anchor) = anchors.last()
        && last_line > last_anchor
    {
        out.push(FoldingRange {
            start_line: last_anchor,
            start_character: None,
            end_line: last_line,
            end_character: None,
            kind: Some(FoldingRangeKind::Region),
            collapsed_text: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_source_yields_no_ranges() {
        assert!(folding_ranges("").is_empty());
    }

    #[test]
    fn single_block_pair_emits_one_range() {
        let src = "本文\n［＃ここから字下げ］\n字下げ内\n［＃ここで字下げ終わり］\n後\n";
        let ranges = folding_ranges(src);
        assert_eq!(ranges.len(), 1, "{ranges:?}");
        assert_eq!(ranges[0].start_line, 1);
        assert_eq!(ranges[0].end_line, 3);
    }

    #[test]
    fn nested_blocks_pair_innermost_first() {
        let src = "［＃ここから罫囲い］\n外\n［＃ここから字下げ］\n中\n［＃ここで字下げ終わり］\n外2\n［＃ここで罫囲い終わり］\n";
        let ranges = folding_ranges(src);
        assert_eq!(ranges.len(), 2);
        let inner = ranges
            .iter()
            .find(|r| r.start_line == 2)
            .expect("inner block");
        assert_eq!(inner.end_line, 4);
        let outer = ranges
            .iter()
            .find(|r| r.start_line == 0)
            .expect("outer block");
        assert_eq!(outer.end_line, 6);
    }

    #[test]
    fn unbalanced_opener_is_silently_dropped() {
        let src = "［＃ここから字下げ］\n中身だけ\n";
        let ranges = folding_ranges(src);
        assert!(ranges.is_empty(), "{ranges:?}");
    }

    #[test]
    fn unbalanced_closer_is_silently_dropped() {
        let src = "中身だけ\n［＃ここで字下げ終わり］\n";
        let ranges = folding_ranges(src);
        assert!(ranges.is_empty(), "{ranges:?}");
    }

    #[test]
    fn heading_lines_pair_with_next_heading() {
        let src =
            "［＃「序章」は大見出し］\n本文1\n本文2\n［＃「第一章」は中見出し］\n本文3\n本文4\n";
        let ranges = folding_ranges(src);
        assert_eq!(ranges.len(), 2);
        let first = &ranges[0];
        assert_eq!(first.start_line, 0);
        assert_eq!(first.end_line, 2);
        let tail = &ranges[1];
        assert_eq!(tail.start_line, 3);
        assert_eq!(tail.end_line, 5);
    }

    #[test]
    fn ranges_are_sorted_by_start_line() {
        let src = "［＃ここから字下げ］\n中\n［＃ここで字下げ終わり］\n後\n［＃「章」は大見出し］\n本文\n";
        let ranges = folding_ranges(src);
        assert!(
            ranges
                .windows(2)
                .all(|w| w[0].start_line <= w[1].start_line)
        );
    }

    #[test]
    fn fold_kind_is_region() {
        let src = "［＃ここから字下げ］\n中\n［＃ここで字下げ終わり］";
        let ranges = folding_ranges(src);
        assert_eq!(ranges[0].kind, Some(FoldingRangeKind::Region));
    }

    #[test]
    fn same_line_open_close_emits_no_degenerate_fold() {
        let src = "［＃ここから字下げ］中身［＃ここで字下げ終わり］\n";
        let ranges = folding_ranges(src);
        assert!(ranges.is_empty(), "{ranges:?}");
    }

    #[test]
    fn adjacent_headings_emit_no_degenerate_window_fold() {
        let src = "［＃「章1」は大見出し］\n［＃「章2」は大見出し］\n本文\n";
        let ranges = folding_ranges(src);
        assert_eq!(ranges.len(), 1, "{ranges:?}");
        assert_eq!(ranges[0].start_line, 1);
        assert_eq!(ranges[0].end_line, 2);
    }

    #[test]
    fn heading_on_last_line_emits_no_tail_fold() {
        let src = "本文\n［＃「章」は大見出し］\n";
        let ranges = folding_ranges(src);
        assert!(ranges.is_empty(), "{ranges:?}");
    }
}
