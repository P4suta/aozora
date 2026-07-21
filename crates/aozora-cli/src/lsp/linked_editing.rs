//! `textDocument/linkedEditingRange` handler — tree-free source scan.
//!
//! When the cursor sits on a recognised opener or closer, return
//! both endpoints as a `LinkedEditingRanges` so the editor mirrors
//! edits between them (type a replacement on `《` and `》` updates
//! too).
//!
//! ## Why source-scan instead of tree-sitter or the Rust parser
//!
//! The original implementation walked the semantic parser's pair table
//! — accurate but cost a full re-parse per cursor move (~414 ms on
//! 40 KB docs). Tree-sitter would let us walk pairs cheaply, but
//! the bracket scan we need is genuinely *local*: from the cursor
//! we look ≤ 1 KB in each direction for the matching delimiter.
//! That's `O(window)` regardless of document size, no parser
//! required, no incremental tree to maintain.
//!
//! ## Coverage
//!
//! Mirrors the four pair shapes most useful to aozora typesetters:
//!
//! - `［` ↔ `］`  bracket (used in `［＃...］` slugs and free brackets)
//! - `《` ↔ `》`  ruby reading delimiter
//! - `「` ↔ `」`  quote
//! - `〔` ↔ `〕`  accent decomposition
//!
//! ASCII `[` ↔ `]` and `(` ↔ `)` etc. are deliberately not handled
//! — those are normal code-style brackets, not aozora notation, and
//! linking them surprises typists writing English in the same buffer.
//!
//! ## Nesting
//!
//! Aozora notation does not nest these pairs (slug bodies do not
//! contain other slugs, ruby readings do not contain ruby). The
//! scan therefore picks the first unbalanced match — adequate for
//! every well-formed corpus document.

use tower_lsp::lsp_types::{LinkedEditingRanges, Position, Range};

use crate::lsp::line_index::LineIndex;

/// Recognised bracket pairs. Order does not matter for lookup.
const PAIRS: &[(char, char)] = &[('［', '］'), ('《', '》'), ('「', '」'), ('〔', '〕')];

/// Maximum look-window (in bytes) for the matching delimiter scan.
/// Aozora slugs / rubies / quotes never span hundreds of bytes; 1 KB
/// covers every realistic case while keeping the scan O(1).
const SCAN_WINDOW: usize = 1024;

/// Return the linked open/close range pair containing `position`, if
/// any. `None` if the cursor is not on a recognised delimiter.
#[must_use]
pub(super) fn linked_editing_at(
    source: &str,
    line_index: &LineIndex,
    position: Position,
) -> Option<LinkedEditingRanges> {
    let cursor = line_index.byte_offset(source, position)?;

    // The "cursor on a delimiter" check has two interpretations: the
    // cursor sits ON the char (byte_offset == start), or JUST AFTER
    // it (byte_offset == end). VS Code's selection model puts the
    // cursor "between" chars, so we accept both — scanning the char
    // immediately before the cursor and the char at the cursor.
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

/// `(byte_start, ch, byte_end)` for the char *at* `offset`, if `offset`
/// sits on a UTF-8 boundary inside `source`.
fn char_at_offset(source: &str, offset: usize) -> Option<(usize, char, usize)> {
    if offset >= source.len() || !source.is_char_boundary(offset) {
        return None;
    }
    let ch = source[offset..].chars().next()?;
    Some((offset, ch, offset + ch.len_utf8()))
}

/// `(byte_start, ch, byte_end)` for the char *immediately before*
/// `offset`. None at the start of the buffer.
fn char_before_offset(source: &str, offset: usize) -> Option<(usize, char, usize)> {
    let head = source.get(..offset)?;
    let (start, ch) = head.char_indices().next_back()?;
    Some((start, ch, offset))
}

/// Test if `(start, ch, end)` is one of the recognised delimiters
/// and find its partner via a bounded scan in `source`. Builds the
/// `LinkedEditingRanges` on hit, `None` on miss.
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

/// Walk forward from `start` looking for `target`. Stops at the
/// scan window or a newline (aozora delimiters never span lines).
///
/// The `SCAN_WINDOW` cap is enforced as a *byte distance* on the
/// fly rather than as a pre-computed end offset — the latter could
/// land mid-codepoint when the window cuts a multi-byte char in
/// half (regression: `&source[..mid_codepoint]` panics).
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

/// Walk backward from `end` (exclusive) looking for `target`.
///
/// `floor` is snapped to the next valid UTF-8 boundary so a
/// `SCAN_WINDOW` saturating-sub that lands inside a multi-byte
/// codepoint does not poison the upcoming slice (regression:
/// `&source[mid_codepoint..end]` panics).
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
        // ASCII `[` and `]` belong to typed code, not aozora notation.
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

    /// Regression: a multi-byte filler whose character-count puts the
    /// `start + SCAN_WINDOW` cap *inside* a multi-byte codepoint used
    /// to panic with "byte index N is not a char boundary; it is
    /// inside 'あ'". Pin: forward scan must respect UTF-8 boundaries
    /// even when the cap falls mid-codepoint.
    #[test]
    fn forward_scan_does_not_panic_on_mid_codepoint_window_cap() {
        // `あ` is 3 UTF-8 bytes; SCAN_WINDOW (1024) is not divisible
        // by 3, so the cap deliberately lands inside an `あ`.
        let filler = "あ".repeat(SCAN_WINDOW); // > SCAN_WINDOW bytes
        let src = format!("《{filler}》");
        let open_byte = src.find('《').unwrap();
        // No matching close within the window — the function must
        // return `None`, not panic.
        let result = linked_editing_at(&src, &LineIndex::new(&src), pos(&src, open_byte));
        assert!(result.is_none(), "expected None, got {result:?}");
    }

    /// Backward scan with `floor = end - SCAN_WINDOW` landing inside
    /// a multi-byte char must not panic on the slice; the floor
    /// snaps to the nearest char boundary.
    #[test]
    fn backward_scan_does_not_panic_on_mid_codepoint_floor() {
        // Same idea but mirrored: cursor on the close brace, with
        // multi-byte filler before it that pushes the backward floor
        // into a codepoint.
        let filler = "あ".repeat(SCAN_WINDOW);
        let src = format!("《{filler}》");
        let close_byte = src.rfind('》').unwrap();
        let result = linked_editing_at(&src, &LineIndex::new(&src), pos(&src, close_byte));
        assert!(result.is_none(), "expected None, got {result:?}");
    }

    /// Forward scan still finds a partner that sits within the window
    /// even when multi-byte chars sit between the cursor and the
    /// close — the boundary fix must not regress the happy path.
    #[test]
    fn forward_scan_finds_partner_through_multibyte_filler() {
        let filler = "あ".repeat(50); // 150 bytes < SCAN_WINDOW
        let src = format!("《{filler}》");
        let open_byte = src.find('《').unwrap();
        let result =
            linked_editing_at(&src, &LineIndex::new(&src), pos(&src, open_byte)).expect("link");
        let close_byte = src.rfind('》').unwrap();
        assert_eq!(result.ranges[1].start, pos(&src, close_byte));
    }

    /// Kills `char_at_offset` `||`→`&&`. A mid-codepoint
    /// `offset` (strictly less than `len`, not a boundary) must be
    /// rejected as `None`. Under `&&` the early return is skipped and
    /// `source[offset..]` panics on the non-boundary slice.
    #[test]
    fn char_at_offset_rejects_mid_codepoint_offset() {
        // In `あい`, `あ` occupies bytes 0..3, so byte 1 is inside a
        // codepoint yet strictly below the length (6).
        assert!(char_at_offset("あい", 1).is_none());
    }

    /// Kills `find_partner_forward` `<`→`<=` on the window bound.
    /// A `target` sitting exactly `SCAN_WINDOW` bytes
    /// from `start` is outside the half-open window and must not be
    /// found; `<=` would wrongly admit it.
    #[test]
    fn forward_scan_excludes_char_exactly_at_window_edge() {
        let src = format!("{}》", "x".repeat(SCAN_WINDOW));
        // `》` starts at byte `SCAN_WINDOW`; distance from start (0) is
        // exactly `SCAN_WINDOW`, so the strict `<` bound rejects it.
        assert_eq!(find_partner_forward(&src, 0, '》'), None);
    }

    /// Kills `find_partner_forward` `idx - start`→`idx + start`.
    /// The window is the distance walked *from*
    /// `start`, so a large `start` with the partner right beside it is
    /// still inside the window and found; `idx + start` would exceed
    /// it spuriously.
    #[test]
    fn forward_scan_measures_distance_from_start_not_sum() {
        let src = format!("{}》", "x".repeat(700));
        // Distance walked is 0 (partner at `start`), well inside the
        // window; `idx + start` = 1400 would falsely blow the window.
        assert_eq!(find_partner_forward(&src, 700, '》'), Some((700, 703)));
    }

    /// Kills `find_partner_forward` end offset `idx + ch.len_utf8()`
    /// mutated to `idx - len` or `idx * len`. The
    /// partner span end must be exactly `idx` plus the char's UTF-8
    /// length.
    #[test]
    fn forward_scan_partner_end_is_start_plus_char_len() {
        // `》` starts at byte 3 and is 3 bytes → span (3, 6).
        // `idx - len` gives (3, 0); `idx * len` gives (3, 9).
        assert_eq!(find_partner_forward("xxx》", 0, '》'), Some((3, 6)));
    }

    /// Kills `find_partner_backward` `floor += 1`→`floor -= 1`.
    /// The floor must snap *up* to the next boundary,
    /// dropping a char that straddles the `end - SCAN_WINDOW` mark.
    #[test]
    fn backward_scan_snaps_floor_up_to_boundary() {
        // `《` occupies bytes 3..6; `end` = 1028 puts
        // `end - SCAN_WINDOW` = 4 mid-`《`. Snapping up to byte 6 drops
        // `《` from the head → no partner. Snapping *down* to byte 3
        // would wrongly include it and yield `Some((3, 6))`.
        let src = format!("xxx《{}", "x".repeat(1022));
        assert_eq!(src.len(), 1028);
        assert_eq!(find_partner_backward(&src, src.len(), '《'), None);
    }

    /// Kills `find_partner_backward` end offset `abs + ch_len` mutated
    /// to `abs - ch_len` or `abs * ch_len`. The partner
    /// span end must be exactly `abs` plus the char's UTF-8 length.
    #[test]
    fn backward_scan_partner_end_is_abs_plus_char_len() {
        // `《` occupies bytes 3..6 → span (3, 6). `abs - len` gives
        // (3, 0); `abs * len` gives (3, 9).
        assert_eq!(find_partner_backward("yyy《xx", 8, '《'), Some((3, 6)));
    }
}
