//! `textDocument/foldingRange` — collapse `［＃ここから…］ … ［＃…終わり］`
//! container blocks and consecutive blank-line stretches.
//!
//! ## Why text-scan rather than tree-sitter
//!
//! Tree-sitter's grammar tokenises `［＃…］` as a slug node but does
//! NOT semantically pair `ここから` / `終わり` markers. The aozora
//! semantic parser does, but it's too slow for a per-cursor request
//! like folding ranges. Direct text scanning against the snapshot's
//! `Arc<str>` is `O(n)` once per request, no parser invoked, and reuses
//! the same byte-then-line conversion pipeline as the rest of the LSP.
//!
//! Two shapes of folds are emitted:
//!
//! 1. **Container blocks** — every line containing `［＃ここから`
//!    pairs with the next line containing `終わり］`. Common pairs:
//!    `字下げ`, `段組`, `小さい字`, `罫囲い`. We just match by the
//!    `ここから … 終わり` shape; the editor doesn't need to know
//!    semantic meaning to fold.
//! 2. **Heading sections** — every line ending in `見出し］` opens
//!    a fold that runs to the next heading-or-end-of-document.

use tower_lsp::lsp_types::{FoldingRange, FoldingRangeKind};

/// Compute every folding range for `source`.
///
/// `source` is taken as a snapshot's `Arc<str>` (immutable for the
/// duration of the call); ranges are returned in source order.
#[must_use]
pub fn folding_ranges(source: &str) -> Vec<FoldingRange> {
    let mut out = Vec::new();
    block_ranges(source, &mut out);
    heading_ranges(source, &mut out);
    out.sort_by_key(|r| r.start_line);
    out
}

/// Pair every `［＃ここから…］` opener with the next `…終わり］`
/// closer — first-in-first-out via a stack so nested containers
/// (e.g. 字下げ inside 罫囲い) match correctly.
fn block_ranges(source: &str, out: &mut Vec<FoldingRange>) {
    const OPEN: &str = "［＃ここから";
    const CLOSE: &str = "終わり］";
    let mut stack: Vec<u32> = Vec::new();
    for (line_idx, line) in source.lines().enumerate() {
        let line_idx = u32::try_from(line_idx).unwrap_or(u32::MAX);
        // A line may contain multiple openers/closers in unusual
        // input; iterate per match.
        let mut search_from = 0usize;
        while let Some(rel) = line[search_from..].find(OPEN) {
            stack.push(line_idx);
            search_from += rel + OPEN.len();
        }
        let mut search_from = 0usize;
        while let Some(rel) = line[search_from..].find(CLOSE) {
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
            search_from += rel + CLOSE.len();
        }
    }
    // Unbalanced openers (open without close) are silently dropped —
    // the editor doesn't show partial folds for them.
}

/// Find every heading marker line and pair each with the next
/// heading line (or end-of-document) so chapter folding works.
///
/// We treat any line containing `見出し］` as a heading anchor.
/// The `aozora` notation supports 大 / 中 / 小 / `同行` variants;
/// all of them end with `見出し］` and we emit folds for the union.
fn heading_ranges(source: &str, out: &mut Vec<FoldingRange>) {
    const HEADING_MARKER: &str = "見出し］";
    let last_line = u32::try_from(source.lines().count().saturating_sub(1)).unwrap_or(0);
    let mut anchors: Vec<u32> = Vec::new();
    for (line_idx, line) in source.lines().enumerate() {
        if line.contains(HEADING_MARKER) {
            anchors.push(u32::try_from(line_idx).unwrap_or(u32::MAX));
        }
    }
    // Pair anchor[i] with anchor[i+1] - 1 (or last_line for the tail).
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
        // Innermost (字下げ) — line 2..4
        let inner = ranges
            .iter()
            .find(|r| r.start_line == 2)
            .expect("inner block");
        assert_eq!(inner.end_line, 4);
        // Outer (罫囲い) — line 0..6
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
        // Two heading anchors → one fold (anchor 0 to anchor 1 - 1 = line 2)
        // + tail fold (anchor 1 to last_line = line 5).
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
        // Block (start 0) + heading (start 4)
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

    /// Opener and closer on the SAME line must NOT emit a degenerate
    /// zero-height fold. Pins `line_idx > start_line` (line 57): with
    /// `>=` the equal-line case (0 == 0) would produce a `start==end`
    /// fold, so the empty result would break.
    #[test]
    fn same_line_open_close_emits_no_degenerate_fold() {
        let src = "［＃ここから字下げ］中身［＃ここで字下げ終わり］\n";
        let ranges = folding_ranges(src);
        assert!(ranges.is_empty(), "{ranges:?}");
    }

    /// Two adjacent heading lines make the window `end == start` (both
    /// resolve to line 0). Pins `end > start` (line 94): with `>=` a
    /// degenerate `start_line == end_line == 0` window fold would be
    /// pushed on top of the tail fold, making the count 2.
    #[test]
    fn adjacent_headings_emit_no_degenerate_window_fold() {
        let src = "［＃「章1」は大見出し］\n［＃「章2」は大見出し］\n本文\n";
        let ranges = folding_ranges(src);
        // Only the tail fold (anchor 1 → last_line 2) survives.
        assert_eq!(ranges.len(), 1, "{ranges:?}");
        assert_eq!(ranges[0].start_line, 1);
        assert_eq!(ranges[0].end_line, 2);
    }

    /// A heading on the very last line makes `last_line == last_anchor`.
    /// Pins `last_line > last_anchor` (line 106): with `>=` a degenerate
    /// `start_line == end_line == 1` tail fold would be emitted.
    #[test]
    fn heading_on_last_line_emits_no_tail_fold() {
        let src = "本文\n［＃「章」は大見出し］\n";
        let ranges = folding_ranges(src);
        assert!(ranges.is_empty(), "{ranges:?}");
    }
}
