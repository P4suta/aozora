//! HTML rendering — `parse`-and-render plus direct-from-artifacts.
//!
//! # Algorithm
//!
//! Single forward `match_indices` sweep over the lexer's normalized
//! text, capturing every structural character (the four PUA
//! sentinels + `\n`) in `O(n)` with no intermediate allocations
//! between matches. Plain text between matches is bulk-copied via
//! `push_str` (one `memcpy` per run) so the inner loop stays
//! branch-predictable on long Japanese prose runs where every
//! character is 3 UTF-8 bytes and a char-by-char walk would pay 3×
//! the bookkeeping cost.
//!
//! HTML escaping inside each plain chunk is handled by a second,
//! nested `match_indices` over `<>&"'` — same bulk-copy strategy,
//! same complexity. The two passes share no state, so the inner
//! escape pass can be inlined or extracted without affecting the
//! outer block walker.
//!
//! Block structure is encoded as an explicit two-state machine
//! [`RenderState::Outside`] ↔ [`RenderState::InParagraph`]; the
//! transitions live on a single method that always emits
//! `<p>` / `</p>` symmetrically, so the output is well-formed by
//! construction. Container nesting is delegated to
//! [`crate::aozora::html::render`] (it emits an opening `<div>` on
//! enter and a closing `</div>` on exit, which we drive by querying
//! the registry at the open and close sentinel positions).
//!
//! # Block model
//!
//! - Two consecutive newlines (`\n\n`) close the current paragraph.
//! - A single newline inside a paragraph emits `<br />`.
//! - `BLOCK_LEAF_SENTINEL` (`U+E002`) flushes the paragraph and
//!   emits the corresponding standalone block element.
//! - `BLOCK_OPEN_SENTINEL` / `BLOCK_CLOSE_SENTINEL` (`U+E003` /
//!   `U+E004`) flush the paragraph and emit the container's
//!   opening / closing tag respectively.
//! - `INLINE_SENTINEL` (`U+E001`) inside a paragraph dispatches the
//!   inline node into the current paragraph buffer.

use aozora_lexer::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL,
};
use aozora_syntax::{AozoraNode, Container};

use crate::aozora::html::render as render_node;
use crate::{ParseArtifacts, parse};

/// Render `input` to HTML.
///
/// Convenience wrapper: runs the lexer and emits HTML in a single
/// pass over the normalized text.
///
/// # Panics
///
/// Does not panic on its own. The lexer's Phase 0 sanitize caps input
/// length at `u32::MAX` bytes, which guarantees every `match_indices`
/// position fits in `u32`; if that invariant is ever broken upstream,
/// the index conversion would panic.
#[must_use]
pub fn render_to_string(input: &str) -> String {
    render_from_artifacts(&parse(input).artifacts)
}

/// Render directly from a [`ParseArtifacts`] bundle.
///
/// Useful for callers that already retained artifacts from a prior
/// [`parse`] (LSP backends, watch-mode tools that diff renders).
///
/// # Panics
///
/// Does not panic on its own. See [`render_to_string`] for the
/// upstream length invariant.
#[must_use]
pub fn render_from_artifacts(artifacts: &ParseArtifacts) -> String {
    let normalized = &artifacts.normalized;
    let registry = &artifacts.registry;

    // Capacity heuristic: plain prose adds `<p>`/`</p>\n` per
    // paragraph (~7 bytes), HTML escape is ~5% on natural text, and
    // each inline sentinel expands to ~50 bytes of markup. We can't
    // know the sentinel count without scanning, so settle on a
    // conservative 2× factor; reallocation cost is amortised by
    // `String::push_str`'s exponential growth.
    let mut out = String::with_capacity(normalized.len().saturating_mul(2));
    let mut state = RenderState::default();

    let bytes = normalized.as_bytes();
    let mut cursor = 0usize;

    for (pos, match_str) in normalized.match_indices(is_structural_char) {
        // Bulk-copy the plain chunk between the previous cursor and
        // this match into the current paragraph (opening one if
        // needed). One `memcpy` per chunk, escape pass over each.
        if cursor < pos {
            state.ensure_in_paragraph(&mut out);
            escape_text_chunk(&normalized[cursor..pos], &mut out);
        }

        // Single-codepoint sentinels — first char is the only char.
        let ch = match_str
            .chars()
            .next()
            .expect("match_indices yields non-empty match");
        let byte_pos = u32::try_from(pos).expect("normalized fits u32 per Phase 0 cap");

        match ch {
            INLINE_SENTINEL => {
                if let Some(node) = registry.inline_at(byte_pos) {
                    state.ensure_in_paragraph(&mut out);
                    let _drop = render_node(node, true, &mut out);
                }
            }
            BLOCK_LEAF_SENTINEL => {
                if let Some(node) = registry.block_leaf_at(byte_pos) {
                    state.before_block_emit(&mut out);
                    let _drop = render_node(node, true, &mut out);
                    state.after_block_emit();
                }
            }
            BLOCK_OPEN_SENTINEL => {
                if let Some(kind) = registry.block_open_at(byte_pos) {
                    state.before_block_emit(&mut out);
                    let node = AozoraNode::Container(Container { kind });
                    let _drop = render_node(&node, true, &mut out);
                    state.after_block_emit();
                }
            }
            BLOCK_CLOSE_SENTINEL => {
                if let Some(kind) = registry.block_close_at(byte_pos) {
                    state.before_block_emit(&mut out);
                    let node = AozoraNode::Container(Container { kind });
                    let _drop = render_node(&node, false, &mut out);
                    state.after_block_emit();
                }
            }
            '\n' => {
                // `\n\n` closes the current paragraph; bare `\n`
                // inside a paragraph is a hard break, except when it
                // is the *trailing* `\n` of the buffer — in that case
                // the post-loop flush will close the paragraph and a
                // `<br />` would dangle at the end of the line. Peek
                // the next byte directly; it is ASCII either way so
                // the byte view is safe.
                match bytes.get(pos + 1) {
                    Some(&b'\n') => state.close_paragraph(&mut out),
                    Some(_) if state.in_paragraph => out.push_str("<br />\n"),
                    Some(_) | None => {}
                }
            }
            _ => unreachable!("is_structural_char admitted only sentinels and \\n"),
        }
        cursor = pos + match_str.len();
    }

    // Flush trailing plain chunk and close any dangling paragraph.
    if cursor < normalized.len() {
        state.ensure_in_paragraph(&mut out);
        escape_text_chunk(&normalized[cursor..], &mut out);
    }
    state.close_paragraph(&mut out);
    out
}

/// Block-level walker state.
///
/// Two flags drive the structural emission:
///
/// - `in_paragraph` — a `<p>` has been opened and is still waiting
///   for `</p>`. `close_paragraph` is the only path that emits the
///   close tag (always with the trailing `\n` separator).
/// - `pending_block_separator` — a non-paragraph block element was
///   the most recent block emission. Block elements rendered by
///   [`crate::aozora::html::render`] produce no trailing newline,
///   so this flag tells the next block emission (a paragraph open
///   or another standalone block) to prepend a separator `\n` and
///   reset itself. Trailing `\n` in the output buffer therefore
///   appears only when more content follows; a standalone block
///   element finishes without dangling whitespace.
///
/// Together they make the renderer's spacing convention match the
/// fixtures exactly without lookahead: each block element decides
/// "do I need a separator before me?" based on what came before,
/// not what comes after.
#[derive(Debug, Default)]
struct RenderState {
    in_paragraph: bool,
    pending_block_separator: bool,
}

impl RenderState {
    fn flush_pending_separator(&mut self, out: &mut String) {
        if self.pending_block_separator {
            out.push('\n');
            self.pending_block_separator = false;
        }
    }

    fn ensure_in_paragraph(&mut self, out: &mut String) {
        if !self.in_paragraph {
            self.flush_pending_separator(out);
            out.push_str("<p>");
            self.in_paragraph = true;
        }
    }

    fn close_paragraph(&mut self, out: &mut String) {
        if self.in_paragraph {
            out.push_str("</p>\n");
            self.in_paragraph = false;
            // Paragraph close already emitted its own `\n`; the next
            // block does not need an extra separator.
            self.pending_block_separator = false;
        }
    }

    fn before_block_emit(&mut self, out: &mut String) {
        self.close_paragraph(out);
        self.flush_pending_separator(out);
    }

    fn after_block_emit(&mut self) {
        self.pending_block_separator = true;
    }
}

/// Predicate matched by the outer `match_indices` sweep.
///
/// Inlined as an `#[inline]` pure function so the optimiser can fold
/// it into the `match_indices` adapter — letting LLVM lower the scan
/// to a `memchr`-class implementation when the predicate is small
/// enough.
#[inline]
fn is_structural_char(c: char) -> bool {
    matches!(
        c,
        INLINE_SENTINEL | BLOCK_LEAF_SENTINEL | BLOCK_OPEN_SENTINEL | BLOCK_CLOSE_SENTINEL | '\n'
    )
}

/// Bulk-escape a plain-text slice into the output buffer.
///
/// Uses a nested `match_indices` over the five HTML-unsafe ASCII
/// characters; the typical Japanese-prose chunk hits zero matches
/// and reduces to a single `push_str`. The escape pass shares the
/// outer's bulk-copy discipline.
fn escape_text_chunk(chunk: &str, out: &mut String) {
    let mut cursor = 0usize;
    for (pos, m) in chunk.match_indices(is_html_unsafe_char) {
        out.push_str(&chunk[cursor..pos]);
        let ch = m.chars().next().expect("non-empty match");
        out.push_str(html_entity_for(ch));
        cursor = pos + m.len();
    }
    out.push_str(&chunk[cursor..]);
}

#[inline]
const fn is_html_unsafe_char(c: char) -> bool {
    matches!(c, '<' | '>' | '&' | '"' | '\'')
}

#[inline]
const fn html_entity_for(c: char) -> &'static str {
    match c {
        '<' => "&lt;",
        '>' => "&gt;",
        '&' => "&amp;",
        '"' => "&quot;",
        '\'' => "&#39;",
        _ => "",
    }
}

// Keep the legacy alias working for `aozora-tools` and any in-tree
// example that retained the old name. Free of cost — re-export.
#[doc(hidden)]
pub use render_to_string as render_root_to_string;

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn plain_paragraph_wraps_in_p() {
        let html = render_to_string("Hello.");
        assert_eq!(html, "<p>Hello.</p>\n");
    }

    #[test]
    fn ruby_is_emitted_semantically() {
        let html = render_to_string("｜青梅《おうめ》");
        assert!(html.contains("<ruby>青梅"), "missing ruby tag: {html}");
        assert!(html.contains("<rt>おうめ"), "missing rt tag: {html}");
    }

    #[test]
    fn page_break_bracket_annotation_emits_page_break_div() {
        let html = render_to_string("前\n\n［＃改ページ］\n\n後");
        assert!(
            html.contains(r#"<div class="afm-page-break"></div>"#),
            "missing page-break div: {html}"
        );
        assert!(!html.contains("［＃"), "［＃ leaked: {html}");
    }

    #[test]
    fn unknown_bracket_annotation_produces_annotation_wrapper() {
        let html = render_to_string("前［＃ほげふが］後");
        assert!(
            html.contains(r#"class="afm-annotation""#),
            "missing annotation wrapper: {html}"
        );
        assert!(
            !html.contains("［＃ほげふが］") || html.contains(">［＃ほげふが］<"),
            "annotation not consumed: {html}"
        );
    }

    #[test]
    fn html_special_characters_are_escaped() {
        let html = render_to_string("a < b & c > d");
        assert!(html.contains("&lt;"), "< not escaped: {html}");
        assert!(html.contains("&gt;"), "> not escaped: {html}");
        assert!(html.contains("&amp;"), "& not escaped: {html}");
        assert!(!html.contains("<scr"), "raw bytes leaked into HTML: {html}");
    }

    #[test]
    fn double_newline_separates_paragraphs() {
        let html = render_to_string("first\n\nsecond");
        assert_eq!(html, "<p>first</p>\n<p>second</p>\n");
    }

    #[test]
    fn single_newline_in_paragraph_emits_hardbreak() {
        let html = render_to_string("verse line one\nverse line two");
        assert!(html.contains("<br />"), "missing hardbreak: {html}");
        // Only one paragraph: no </p><p> seam.
        assert_eq!(html.matches("<p>").count(), 1);
        assert_eq!(html.matches("</p>").count(), 1);
    }

    #[test]
    fn empty_input_produces_empty_output() {
        assert_eq!(render_to_string(""), "");
    }

    #[test]
    fn run_of_blank_lines_does_not_emit_empty_paragraphs() {
        let html = render_to_string("first\n\n\n\n\nsecond");
        assert_eq!(html.matches("<p>").count(), 2);
        assert_eq!(html.matches("</p>").count(), 2);
        assert!(!html.contains("<p></p>"));
    }
}
