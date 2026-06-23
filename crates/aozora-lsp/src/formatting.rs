//! `textDocument/formatting` handler.
//!
//! Returns a single document-replace [`TextEdit`] when the canonical
//! form differs from the current buffer, and an empty edit list
//! otherwise — which is what `rust-analyzer` and `taplo` do too.
//! That lets LSP clients short-circuit "already formatted" buffers
//! without applying a no-op edit.

use aozora::Document;
use tower_lsp::lsp_types::{Position, Range, TextEdit};

use crate::position::byte_offset_to_position;

/// Compute the list of `TextEdit`s that canonicalise `source`.
///
/// Runs the same `parse ∘ serialize` round-trip as `aozora-fmt` — the
/// canonical form is defined by `aozora` itself, so calling it directly
/// keeps the server's output byte-identical to the formatter without
/// depending on the `aozora-fmt` crate (and its CLI-only dependencies).
#[must_use]
pub fn format_edits(source: &str) -> Vec<TextEdit> {
    let formatted = Document::new(source).parse().to_source();
    if formatted == source {
        return Vec::new();
    }
    let end = byte_offset_to_position(source, source.len());
    vec![TextEdit {
        range: Range::new(Position::new(0, 0), end),
        new_text: formatted,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_input_produces_no_edits() {
        // Bare ruby is canonical (all-kanji base at line start, ADR 0002/0003).
        assert!(format_edits("日本《にほん》").is_empty());
    }

    #[test]
    fn non_canonical_ruby_produces_one_replace_edit() {
        // A redundant explicit ｜ canonicalises to the bare form.
        let src = "｜日本《にほん》";
        let edits = format_edits(src);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start, Position::new(0, 0));
        assert!(
            edits[0].new_text.starts_with("日本《にほん》") && !edits[0].new_text.contains('｜')
        );
    }

    #[test]
    fn empty_source_produces_no_edits() {
        assert!(format_edits("").is_empty());
    }
}
