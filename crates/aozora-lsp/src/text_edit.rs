//! In-tree text edit primitive: turn `(byte_range, new_text)` pairs
//! into a validated string splice.
//!
//! Validation policy:
//!
//! - Edits are processed in source order; an edit's `range` must lie
//!   strictly after every prior edit's `range.end`.
//! - Each `range` must be an in-bounds, non-inverted byte slice on
//!   UTF-8 char boundaries — out-of-range / cross-boundary edits are
//!   rejected wholesale (no partial application).
//! - Empty edits (`range.is_empty() && new_text.is_empty()`) are
//!   permitted but produce no change.

use std::ops::Range;

/// A single text edit: replace `range` in the source with `new_text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteEdit {
    pub range: Range<usize>,
    pub new_text: String,
}

impl ByteEdit {
    #[must_use]
    pub const fn new(range: Range<usize>, new_text: String) -> Self {
        Self { range, new_text }
    }
}

/// Errors returned by [`apply_edits`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EditError {
    #[error("edit range {start}..{end} is inverted")]
    InvertedRange { start: usize, end: usize },
    #[error("edit range {start}..{end} extends past source.len() = {source_len}")]
    OutOfBounds {
        start: usize,
        end: usize,
        source_len: usize,
    },
    #[error("edit range {start}..{end} crosses a UTF-8 char boundary")]
    NonCharBoundary { start: usize, end: usize },
    #[error("edits not sorted: {prev_end} > {next_start}")]
    UnsortedOrOverlapping { prev_end: usize, next_start: usize },
}

/// Splice every `ByteEdit` in `edits` into `source` and return
/// the resulting string.
///
/// `edits` must be sorted by `range.start` and non-overlapping. Each
/// `range` must be in-bounds and aligned to UTF-8 char boundaries.
/// Any violation aborts the splice and leaves no partial output.
///
/// # Errors
///
/// Returns [`EditError`] when an edit is inverted, out-of-bounds,
/// crosses a UTF-8 char boundary, or overlaps a prior edit. No partial
/// application happens — `source` is unchanged on error.
pub fn apply_edits(source: &str, edits: &[ByteEdit]) -> Result<String, EditError> {
    // Pre-validate every edit so we can emit a single Result<String, _>
    // without partial application.
    let mut prev_end = 0usize;
    for edit in edits {
        let Range { start, end } = edit.range;
        if end < start {
            return Err(EditError::InvertedRange { start, end });
        }
        if end > source.len() {
            return Err(EditError::OutOfBounds {
                start,
                end,
                source_len: source.len(),
            });
        }
        if !source.is_char_boundary(start) || !source.is_char_boundary(end) {
            return Err(EditError::NonCharBoundary { start, end });
        }
        if start < prev_end {
            return Err(EditError::UnsortedOrOverlapping {
                prev_end,
                next_start: start,
            });
        }
        prev_end = end;
    }

    // Pre-compute the output capacity: every byte of source plus net
    // delta from each edit.
    let total_new: usize = edits.iter().map(|e| e.new_text.len()).sum();
    let total_removed: usize = edits.iter().map(|e| e.range.len()).sum();
    let cap = source
        .len()
        .saturating_add(total_new)
        .saturating_sub(total_removed);
    let mut out = String::with_capacity(cap);

    let mut cursor = 0usize;
    for edit in edits {
        let Range { start, end } = edit.range;
        // Copy the un-edited slice up to this edit, then the
        // replacement.
        out.push_str(&source[cursor..start]);
        out.push_str(&edit.new_text);
        cursor = end;
    }
    // Trailing tail.
    out.push_str(&source[cursor..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_edits_returns_source_unchanged() {
        let src = "hello, world";
        assert_eq!(apply_edits(src, &[]).unwrap(), src);
    }

    #[test]
    fn single_replacement() {
        let src = "hello, world";
        let edit = ByteEdit::new(7..12, "rust".to_owned());
        assert_eq!(apply_edits(src, &[edit]).unwrap(), "hello, rust");
    }

    #[test]
    fn pure_insertion() {
        let src = "abcdef";
        let edit = ByteEdit::new(3..3, "_X_".to_owned());
        assert_eq!(apply_edits(src, &[edit]).unwrap(), "abc_X_def");
    }

    #[test]
    fn pure_deletion() {
        let src = "abcdef";
        let edit = ByteEdit::new(2..4, String::new());
        assert_eq!(apply_edits(src, &[edit]).unwrap(), "abef");
    }

    #[test]
    fn multiple_sorted_edits_compose() {
        let src = "AAAA BBBB CCCC";
        let edits = vec![
            ByteEdit::new(0..4, "aa".to_owned()),
            ByteEdit::new(5..9, "bb".to_owned()),
            ByteEdit::new(10..14, "cc".to_owned()),
        ];
        assert_eq!(apply_edits(src, &edits).unwrap(), "aa bb cc");
    }

    #[test]
    fn overlapping_edits_fail() {
        let src = "AAAA BBBB";
        let edits = vec![
            ByteEdit::new(0..5, "x".to_owned()),
            ByteEdit::new(3..7, "y".to_owned()),
        ];
        let err = apply_edits(src, &edits).unwrap_err();
        assert!(matches!(err, EditError::UnsortedOrOverlapping { .. }));
    }

    #[test]
    fn out_of_bounds_fail() {
        let err = apply_edits("abc", &[ByteEdit::new(0..99, String::new())]).unwrap_err();
        assert!(matches!(err, EditError::OutOfBounds { .. }));
    }

    #[test]
    fn cross_char_boundary_fail() {
        // 「あ」 is 3 UTF-8 bytes. Range 1..2 sits inside it.
        let err = apply_edits("あ", &[ByteEdit::new(1..2, String::new())]).unwrap_err();
        assert!(matches!(err, EditError::NonCharBoundary { .. }));
    }

    /// `start` sits on a valid boundary but `end` does not — exactly
    /// one operand of the char-boundary guard is true. `「あ」` is 3
    /// bytes (boundaries at 0 and 3); `0..1` has `start == 0` (boundary)
    /// and `end == 1` (mid-char). The unmodified `||` rejects because
    /// `end` is off-boundary; a `&&` mutant would require BOTH endpoints
    /// off-boundary and so would not reject here.
    #[test]
    fn end_only_off_char_boundary_fails() {
        let err = apply_edits("あ", &[ByteEdit::new(0..1, String::new())]).unwrap_err();
        assert!(matches!(
            err,
            EditError::NonCharBoundary { start: 0, end: 1 }
        ));
    }

    /// `end` sits on a valid boundary but `start` does not — the mirror
    /// of the case above, pinning the other operand of the `||`. For
    /// `「あ」`, `1..3` has `start == 1` (mid-char) and `end == 3` (the
    /// trailing boundary, equal to `source.len()`). The unmodified `||`
    /// rejects because `start` is off-boundary; a `&&` mutant would not.
    #[test]
    fn start_only_off_char_boundary_fails() {
        let err = apply_edits("あ", &[ByteEdit::new(1..3, String::new())]).unwrap_err();
        assert!(matches!(
            err,
            EditError::NonCharBoundary { start: 1, end: 3 }
        ));
    }

    #[test]
    fn inverted_range_fail() {
        // Construct an inverted range explicitly — `5..2` would
        // trigger clippy::reversed_empty_ranges, so we build the
        // Range struct from its endpoints.
        let edit = ByteEdit {
            range: Range { start: 5, end: 2 },
            new_text: String::new(),
        };
        let err = apply_edits("hello", &[edit]).unwrap_err();
        assert!(matches!(err, EditError::InvertedRange { .. }));
    }

    #[test]
    fn multibyte_replacement_at_char_boundary() {
        // Replace `あ` (3 bytes) with `い` (3 bytes).
        let src = "あいう";
        let edit = ByteEdit::new(0..3, "い".to_owned());
        assert_eq!(apply_edits(src, &[edit]).unwrap(), "いいう");
    }

    #[test]
    fn empty_range_at_eof_appends() {
        let src = "abc";
        let edit = ByteEdit::new(3..3, "DEF".to_owned());
        assert_eq!(apply_edits(src, &[edit]).unwrap(), "abcDEF");
    }

    /// Two pure inserts at the same offset: both are non-overlapping
    /// (`start < prev_end` is false because both `end`s equal 0), so
    /// validation passes. The forward apply produces them in source
    /// order — first edit's text precedes second's. Pin so any change
    /// to the apply order surfaces here instead of silently re-ordering
    /// inserted content.
    #[test]
    fn two_inserts_at_same_offset_apply_in_source_order() {
        let src = "X";
        let edits = vec![
            ByteEdit::new(0..0, "a".to_owned()),
            ByteEdit::new(0..0, "b".to_owned()),
        ];
        assert_eq!(apply_edits(src, &edits).unwrap(), "abX");
    }

    /// Adjacent edits — second edit starts exactly at first edit's
    /// end. Allowed by the validator (`start < prev_end` is false
    /// when `start == prev_end`). The two edits should compose into
    /// a single replacement-like result.
    #[test]
    fn adjacent_edits_compose() {
        let src = "ABCD";
        let edits = vec![
            ByteEdit::new(0..2, "x".to_owned()),
            ByteEdit::new(2..4, "y".to_owned()),
        ];
        assert_eq!(apply_edits(src, &edits).unwrap(), "xy");
    }

    /// A multi-byte replacement at the start of source must not
    /// affect the trailing tail; pin via a Japanese run so the byte
    /// math (3 bytes per char) is non-trivial.
    #[test]
    fn multibyte_replacement_at_start_preserves_tail() {
        let src = "あいう";
        let edit = ByteEdit::new(0..3, "X".to_owned());
        assert_eq!(apply_edits(src, &[edit]).unwrap(), "Xいう");
    }

    /// Validation arithmetic regression: `total_new` and
    /// `total_removed` are computed from edits to size the output
    /// `String::with_capacity`. A pure-deletion-then-pure-insertion
    /// pair must not under- or over-allocate enough that the
    /// resulting string differs from the expected splice.
    #[test]
    fn delete_then_insert_in_separate_edits_produces_expected_text() {
        let src = "ABCDEF";
        let edits = vec![
            ByteEdit::new(0..2, String::new()),
            ByteEdit::new(4..4, "XYZ".to_owned()),
        ];
        // Remove "AB" (0..2), then insert "XYZ" at 4 (= "F" position).
        assert_eq!(apply_edits(src, &edits).unwrap(), "CDXYZEF");
    }
}
