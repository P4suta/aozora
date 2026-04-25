//! Document segmentation for the parallel parse path.
//!
//! [`identify_segments`] partitions an aozora source string into
//! contiguous byte ranges whose union covers the whole input and each
//! of which can be lexed in isolation by [`aozora_lexer::lex`]. The
//! parallel entry point ([`crate::parse_parallel`]) then runs `lex`
//! on each segment in a rayon thread pool and merges the per-segment
//! outputs.
//!
//! # Independence guarantee
//!
//! A "segment" is independent of its neighbours when:
//!
//! - Its boundaries fall on a paragraph break (one or more consecutive
//!   `\n` characters that delimit a blank line). Phase 0 sanitize is
//!   line-local apart from a leading-BOM strip and inline `〔...〕`
//!   accent decomposition; neither crosses a paragraph break, so the
//!   per-segment Phase 0 output concatenates cleanly to the
//!   whole-document Phase 0 output.
//! - It contains no half-open paired container — i.e. every
//!   `［＃ここから…］` opener within the segment has a matching
//!   `［＃ここで…終わり］`-shaped closer also within the segment.
//!   Paired-container content can span multiple paragraphs (e.g.
//!   `［＃ここから割書］\n\n …\n\n［＃ここで割書終わり］`); we keep
//!   such runs in a single segment so the lexer's Phase 2 pair pass
//!   sees a balanced bracket stack.
//! - It contains no half-open inline bracket. Inline brackets include
//!   `［…］` (annotation), `《…》` (ruby), `〔…〕` (accent segment),
//!   and `「…」` (quote). All are single-line by spec but malformed
//!   inputs sometimes contain a paragraph break inside one; the
//!   sequential lexer reports such cases as one unclosed-bracket
//!   diagnostic, the parallel path would otherwise see them as two
//!   unrelated halves and emit two diagnostics. Tracking total
//!   bracket depth keeps diagnostic counts identical.
//!
//! # Algorithm
//!
//! Single O(n) byte pass:
//!
//! 1. `str::match_indices` finds every paired-container open
//!    (`［＃ここから`) and close (`［＃ここで`). These two prefixes
//!    are the two distinguishing markers in the spec; we don't need
//!    to know which exact container kind, only depth.
//! 2. A blank-line scan finds every `\n\n+` run (one or more blank
//!    lines).
//! 3. Both event streams are merged in source order; depth is updated
//!    on opens / closes, and a paragraph break emits a segment
//!    boundary only when depth has returned to zero.
//!
//! The depth-tracking guard is a *conservative superset*: any
//! byte-level occurrence of `［＃ここから` increments depth even
//! inside, say, an unclosed comment. The cost of the conservatism is
//! at most one fewer segment break — never an incorrect break. The
//! property test in the parallel module pins this against random
//! aozora-shaped input.

use core::ops::Range;

const CONTAINER_OPEN: &str = "［＃ここから";
const CONTAINER_CLOSE: &str = "［＃ここで";

/// Bracket pair kinds — mirrors `aozora_lexer::PairKind` but kept
/// independent here so the segmenter doesn't depend on the lexer's
/// internal type. Order matters only for tie-break determinism in
/// the event-sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum BracketKind {
    Annotation, // ［ ... ］
    Ruby,       // 《 ... 》
    Tortoise,   // 〔 ... 〕
    Quote,      // 「 ... 」
}

const fn bracket_kind_for_char(c: char) -> Option<(BracketKind, bool)> {
    match c {
        '［' => Some((BracketKind::Annotation, true)),
        '］' => Some((BracketKind::Annotation, false)),
        '《' => Some((BracketKind::Ruby, true)),
        '》' => Some((BracketKind::Ruby, false)),
        '〔' => Some((BracketKind::Tortoise, true)),
        '〕' => Some((BracketKind::Tortoise, false)),
        '「' => Some((BracketKind::Quote, true)),
        '」' => Some((BracketKind::Quote, false)),
        _ => None,
    }
}

/// Partition `source` into segments that can each be lexed
/// independently. Each returned range is half-open; their union is
/// `0..source.len()` with no gaps and no overlap.
///
/// For inputs without any paragraph break (single paragraph or empty
/// source) returns a single range covering the whole input.
#[must_use]
pub fn identify_segments(source: &str) -> Vec<Range<usize>> {
    if source.is_empty() {
        return Vec::new();
    }

    // Collect all events into one sorted list. Two depths are
    // tracked independently:
    //
    //   container_depth: `［＃ここから` / `［＃ここで` balance —
    //     keeps a paired-container BODY (which spans paragraphs by
    //     design) inside one segment.
    //   bracket_depth: total open/close balance over `［《〔「` /
    //     `］》〕」` — catches malformed input where a paragraph
    //     break lands inside an unclosed inline bracket pair, which
    //     the sequential lexer reports as one unclosed-bracket
    //     diagnostic but the parallel path would otherwise split
    //     into two unrelated halves.
    //
    // Container opens/closes are detected by their full string
    // prefix, then the surrounding `［` / `］` chars contribute to
    // bracket_depth via the inline scan. Net effect: a well-formed
    // `［＃ここから割書］` opener contributes (+1, +1) then (0, -1)
    // back to (+1, 0) — container depth stays raised through the
    // body, bracket depth returns to 0.
    let mut events: Vec<Event> = Vec::new();
    for (pos, _) in source.match_indices(CONTAINER_OPEN) {
        events.push(Event {
            pos,
            kind: EventKind::ContainerOpen,
        });
    }
    for (pos, _) in source.match_indices(CONTAINER_CLOSE) {
        events.push(Event {
            pos,
            kind: EventKind::ContainerClose,
        });
    }
    for (pos, ch) in source.char_indices() {
        if let Some((kind, is_open)) = bracket_kind_for_char(ch) {
            events.push(Event {
                pos,
                kind: if is_open {
                    EventKind::BracketOpen(kind)
                } else {
                    EventKind::BracketClose(kind)
                },
            });
        }
    }
    for (start, end) in find_paragraph_breaks(source) {
        events.push(Event {
            pos: start,
            kind: EventKind::Break { end },
        });
    }
    // Stable sort by position, then by kind so opens at the same
    // byte apply before closes / breaks.
    events.sort_by(|a, b| a.pos.cmp(&b.pos).then_with(|| a.kind.cmp(&b.kind)));

    let mut segments = Vec::new();
    let mut seg_start = 0usize;
    let mut container_depth: i32 = 0;
    // Typed stack mirrors `aozora_lexer::phase2_pair`'s matcher: a
    // close pops only if the top entry has the same kind. A
    // mismatched close is "stray" and leaves the stack untouched, so
    // the still-unmatched open keeps its paragraph-break-blocking
    // role until a same-kind close appears (or never does — in that
    // case the segment extends to EOF, which is still correct).
    let mut bracket_stack: Vec<BracketKind> = Vec::new();
    for ev in events {
        match ev.kind {
            EventKind::ContainerOpen => container_depth += 1,
            EventKind::ContainerClose => {
                container_depth = (container_depth - 1).max(0);
            }
            EventKind::BracketOpen(kind) => bracket_stack.push(kind),
            EventKind::BracketClose(kind) => {
                if bracket_stack.last() == Some(&kind) {
                    bracket_stack.pop();
                }
            }
            EventKind::Break { end } => {
                if container_depth == 0 && bracket_stack.is_empty() && end > seg_start {
                    segments.push(seg_start..end);
                    seg_start = end;
                }
            }
        }
    }
    if seg_start < source.len() {
        segments.push(seg_start..source.len());
    } else if segments.is_empty() {
        // All-whitespace or single-segment edge case where the loop
        // emitted everything via paragraph breaks; ensure non-empty.
        segments.push(0..source.len());
    }
    segments
}

#[derive(Debug, Clone, Copy)]
struct Event {
    pos: usize,
    kind: EventKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum EventKind {
    // Sort order matters for the tie-break in identify_segments:
    // opens < closes < breaks so opens at the same byte offset
    // apply first. Container events sort before Bracket events for
    // determinism.
    ContainerOpen,
    BracketOpen(BracketKind),
    ContainerClose,
    BracketClose(BracketKind),
    Break { end: usize },
}

/// Find every maximal run of one or more `\n` characters that begins
/// after a non-`\n` byte. Returned tuples are
/// `(run_start, run_end_exclusive)`. A "blank line" run is a run of
/// length ≥ 2; runs of length 1 represent a soft line break inside a
/// paragraph and we still emit them so the segmentation policy can
/// be tuned in one place.
///
/// Only the runs of length ≥ 2 are paragraph breaks. We filter here
/// so callers don't have to.
fn find_paragraph_breaks(source: &str) -> Vec<(usize, usize)> {
    let bytes = source.as_bytes();
    let mut runs = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            let mut j = i;
            while j < bytes.len() && bytes[j] == b'\n' {
                j += 1;
            }
            if j - i >= 2 {
                runs.push((i, j));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_source_yields_no_segments() {
        assert!(identify_segments("").is_empty());
    }

    #[test]
    fn no_paragraph_break_returns_single_segment() {
        let s = "one paragraph only";
        assert_eq!(identify_segments(s), vec![0..s.len()]);
    }

    #[test]
    fn single_line_break_does_not_split() {
        // One `\n` is a soft break inside a paragraph; not a split.
        let s = "line one\nline two";
        assert_eq!(identify_segments(s), vec![0..s.len()]);
    }

    #[test]
    fn double_newline_splits_into_two_segments() {
        let s = "first\n\nsecond";
        let segs = identify_segments(s);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0], 0..7); // "first\n\n"
        assert_eq!(segs[1], 7..s.len()); // "second"
        // Concatenation property: union with no gaps.
        assert_eq!(segs[0].end, segs[1].start);
        assert_eq!(segs[1].end, s.len());
    }

    #[test]
    fn three_paragraphs_split_into_three_segments() {
        let s = "a\n\nb\n\nc";
        let segs = identify_segments(s);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0], 0..3);
        assert_eq!(segs[1], 3..6);
        assert_eq!(segs[2], 6..s.len());
    }

    #[test]
    fn long_blank_line_run_collapses_to_one_break() {
        let s = "a\n\n\n\nb";
        let segs = identify_segments(s);
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].end, 5); // entire run goes into segment 0
        assert_eq!(segs[1], 5..s.len());
    }

    #[test]
    fn paired_container_keeps_paragraphs_together() {
        // Open and close on different paragraphs → must stay in
        // same segment.
        let s = "［＃ここから割書］\n\nbody1\n\nbody2\n\n［＃ここで割書終わり］";
        let segs = identify_segments(s);
        // All in one segment since depth never returns to 0 between
        // paragraph boundaries (until the final close).
        assert_eq!(segs.len(), 1, "got: {segs:?}");
        assert_eq!(segs[0], 0..s.len());
    }

    #[test]
    fn nested_paired_containers_stay_in_one_segment() {
        let s = "［＃ここから割書］\n\
                 ［＃ここから縦中横］\n\n\
                 inner\n\n\
                 ［＃ここで縦中横終わり］\n\n\
                 ［＃ここで割書終わり］";
        let segs = identify_segments(s);
        assert_eq!(segs.len(), 1, "got: {segs:?}");
    }

    #[test]
    fn closed_container_then_paragraph_split_works() {
        // After the close, depth is 0 and the next break splits.
        let s = "［＃ここから割書］inside［＃ここで割書終わり］\n\nafter";
        let segs = identify_segments(s);
        assert_eq!(segs.len(), 2, "got: {segs:?}");
        assert!(segs[0].contains(&0));
        assert_eq!(segs[1].end, s.len());
    }

    #[test]
    fn stray_close_does_not_underflow() {
        // A close without a matching open should not push depth
        // negative; subsequent paragraph breaks must still split.
        let s = "［＃ここで割書終わり］\n\nafter";
        let segs = identify_segments(s);
        assert_eq!(segs.len(), 2, "got: {segs:?}");
    }

    #[test]
    fn segments_cover_source_exactly() {
        // Property: union of segments equals 0..source.len() with no
        // overlap. Manually exercise a varied input.
        let s = "para1\n\n｜漢字《かんじ》\n\nmore\n\n［＃ここから割書］\n\nA\n\n［＃ここで割書終わり］\n\nlast";
        let segs = identify_segments(s);
        assert!(!segs.is_empty());
        assert_eq!(segs.first().unwrap().start, 0);
        assert_eq!(segs.last().unwrap().end, s.len());
        for window in segs.windows(2) {
            assert_eq!(window[0].end, window[1].start, "gap: {window:?}");
        }
    }

    #[test]
    fn break_runs_emit_only_for_two_or_more_newlines() {
        // Just `\n` between two paragraphs is NOT a paragraph break.
        let s = "a\nb\nc";
        assert!(find_paragraph_breaks(s).is_empty());
        let s2 = "a\n\nb";
        assert_eq!(find_paragraph_breaks(s2), vec![(1, 3)]);
    }

    #[test]
    fn unicode_paragraph_with_breaks_keeps_byte_indices() {
        // Confirm that returned ranges are valid byte indices into
        // the original source even with multi-byte UTF-8 content.
        let s = "あ\n\nい\n\nう";
        let segs = identify_segments(s);
        assert_eq!(segs.len(), 3);
        for seg in &segs {
            // Each range must be a valid char-boundary slice.
            assert!(s.is_char_boundary(seg.start));
            assert!(s.is_char_boundary(seg.end));
        }
    }
}
