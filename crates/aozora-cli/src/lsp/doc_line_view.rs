//! Byte-offset → LSP [`Position`] mapping that serves both the
//! rope-backed publish hot path and the `&str`-backed on-demand paths.
//!
//! ## Why this exists
//!
//! Every debounced `publishDiagnostics` used to rebuild a whole-document
//! line table ([`LineIndex::new`](crate::lsp::line_index::LineIndex::new), an `O(doc)` `\n` scan) just to map a
//! handful of diagnostic byte spans onto LSP positions. The document is
//! already paragraph-rope-segmented, so the publish path can assemble a
//! doc-level [`Rope`] by structural-share `append` (no byte copy) and map
//! offsets with `O(log n)` rope line lookups instead — eliminating the
//! per-keystroke `O(doc)` rebuild (#237 Tier-2, Mechanism A).
//!
//! ## Byte-identity with [`LineIndex`](crate::lsp::line_index::LineIndex)
//!
//! [`LineIndex`](crate::lsp::line_index::LineIndex) counts **only** `\n` (`0x0A`) as a line break. ropey's
//! default features (`unicode_lines` → `cr_lines`) would also count CR,
//! VT, FF, NEL, LS, and PS, so the workspace pins ropey with
//! `default-features = false` — leaving only `0x0A` as a break, byte-
//! identical to [`LineIndex`](crate::lsp::line_index::LineIndex). The [`Rope`] arm here reproduces
//! [`LineIndex::position`](crate::lsp::line_index::LineIndex::position) exactly, **including** the pre-existing skew
//! where a diagnostic's span (in sanitized coordinates) is mapped against
//! the raw text; that skew is a separate tracked concern and is preserved
//! verbatim. A non-char-boundary offset panics in both arms identically.
//!
//! The `ropey_counts_only_lf` guard test fails loudly if the ropey feature
//! flip is ever reverted.

use ropey::Rope;
use tower_lsp::lsp_types::Position;

#[cfg(test)]
use crate::lsp::line_index::LineIndex;

/// A byte-offset → [`Position`] mapper over one of two backings.
///
/// Either a doc-level [`Rope`] (the publish hot path, `O(log n)` per
/// lookup, no line-table rebuild) or a borrowed `&str` plus its
/// [`LineIndex`](crate::lsp::line_index::LineIndex) (the on-demand paths and the one-shot
/// `diagnostics_for_source`, identical to today's behaviour). Both arms
/// produce byte-identical positions; see the module docs.
#[derive(Debug)]
pub(super) enum DocLineView<'a> {
    /// Doc-level rope assembled by structural-share `append`. Line
    /// metrics count only `0x0A` (workspace ropey has its default
    /// `unicode_lines`/`cr_lines` features disabled).
    Rope(&'a Rope),
    /// Borrowed source plus its line index — the verbatim [`LineIndex`](crate::lsp::line_index::LineIndex)
    /// path for callers that already hold a `&str`. Test-only: the server
    /// always assembles a doc-level rope (`Rope`); the `&str` arm backs the
    /// in-module tests that cross-check the two paths agree.
    #[cfg(test)]
    Str {
        /// The source text the offsets index into.
        source: &'a str,
        /// Precomputed line-start table for `source`.
        index: LineIndex,
    },
}

/// Build a `&str`-backed view, computing the [`LineIndex`](crate::lsp::line_index::LineIndex) once. Test-only
/// (the `Str` arm it constructs exists only under `cfg(test)`); kept in its own
/// lifetime-named impl so the live `position` impl below stays `'_`.
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
    /// Map `byte_offset` (clamped to the document length) onto an LSP
    /// [`Position`]. Byte-identical across both arms.
    #[must_use]
    pub(super) fn position(&self, byte_offset: usize) -> Position {
        match self {
            #[cfg(test)]
            Self::Str { source, index } => index.position(source, byte_offset),
            Self::Rope(rope) => rope_position(rope, byte_offset),
        }
    }
}

/// Reproduce [`LineIndex::position`](crate::lsp::line_index::LineIndex::position) exactly over a LF-only [`Rope`].
///
/// `byte_to_line` assigns the `\n` to its own (left) line, matching
/// [`LineIndex`](crate::lsp::line_index::LineIndex)'s `partition_point(start <= needle) - 1`; the column is
/// the UTF-16 width of the bytes from the line start up to `byte_offset`.
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

    /// Load-bearing guard: with ropey's default features disabled, line
    /// metrics count **only** `0x0A`. If the `default-features = false`
    /// flip in the workspace `Cargo.toml` is ever reverted, ropey would
    /// count CR / VT / FF / NEL / LS / PS as breaks and the `Rope` arm
    /// would diverge from `LineIndex` (which counts only `\n`). This
    /// string packs one of each non-LF separator and must stay a single
    /// line.
    #[test]
    fn ropey_counts_only_lf() {
        let r = Rope::from("a\rb\u{0B}c\u{0C}d\u{85}e\u{2028}f\u{2029}g");
        assert_eq!(r.len_lines(), 1);
    }

    /// The `Rope` arm reproduces `LineIndex::position` for a CRLF doc,
    /// including the `\r`-stays-inside-the-line behaviour.
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

    /// The `Str` arm is the verbatim `LineIndex` path.
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

    /// Overshoot clamps to EOF in the `Rope` arm, matching `LineIndex`.
    #[test]
    fn rope_arm_overshoot_clamps_to_eof() {
        let rope = Rope::from("hi");
        assert_eq!(DocLineView::Rope(&rope).position(99), Position::new(0, 2));
    }
}
