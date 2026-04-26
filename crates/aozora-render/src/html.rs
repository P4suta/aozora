//! Borrowed-AST HTML rendering.
//!
//! Mirrors `aozora_parser::html::render_from_artifacts_to_writer` but
//! consumes a [`BorrowedLexOutput`] (Plan B.2 output) directly. The
//! algorithm and emitted bytes are identical; only the input shape and
//! per-node renderer change.
//!
//! # Algorithm
//!
//! Single forward `match_indices` sweep over the lexer's normalized
//! text, capturing every PUA sentinel + `\n` in O(n). Plain text
//! between matches flows through the bulk-copy escape pass; sentinels
//! dispatch into [`crate::render_node::render`] via the borrowed
//! registry's [`EytzingerMap::get`] lookup.
//!
//! Block structure mirrors the legacy walker: a two-state machine
//! [`RenderState::ensure_in_paragraph`] / [`RenderState::close_paragraph`]
//! emits `<p>` / `</p>` symmetrically; standalone block nodes (and
//! container open/close events) flush the current paragraph first.

use core::fmt;

use aozora_lex::BorrowedLexOutput;
use aozora_spec::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL,
};
use aozora_syntax::Container;
use aozora_syntax::borrowed::AozoraNode;

use crate::render_node;

/// Render a `BorrowedLexOutput` into a fresh `String`.
///
/// Allocates roughly `2 × normalized.len()` upfront — the same growth
/// strategy as `aozora_parser::html::render_to_string`. For streaming
/// consumers prefer [`render_into`] to avoid the intermediate `String`.
///
/// # Panics
///
/// Does not panic in normal use: `String` cannot fail as a
/// [`fmt::Write`] sink. The internal `expect` covers the trivially
/// unreachable case.
#[must_use]
pub fn render_to_string(out: &BorrowedLexOutput<'_>) -> String {
    let mut s = String::with_capacity(out.normalized.len().saturating_mul(2));
    render_into(out, &mut s).expect("writing to String never fails");
    s
}

/// Render a `BorrowedLexOutput` into the given writer.
///
/// # Errors
///
/// Propagates write errors from `writer`.
///
/// # Panics
///
/// Panics if the normalized text exceeds `u32::MAX` bytes — inherited
/// from the lexer's `Span` width contract; in practice unreachable
/// (Phase 0 sanitize already gates on this bound).
pub fn render_into<W: fmt::Write>(out: &BorrowedLexOutput<'_>, writer: &mut W) -> fmt::Result {
    let normalized = out.normalized;
    let registry = &out.registry;
    let mut state = RenderState::default();

    let bytes = normalized.as_bytes();
    let mut cursor = 0usize;

    for (pos, match_str) in normalized.match_indices(is_structural_char) {
        if cursor < pos {
            state.ensure_in_paragraph(writer)?;
            escape_text_chunk(&normalized[cursor..pos], writer)?;
        }

        let ch = match_str
            .chars()
            .next()
            .expect("match_indices yields non-empty match");
        let byte_pos = u32::try_from(pos).expect("normalized fits u32 per Phase 0 cap");

        match ch {
            INLINE_SENTINEL => {
                if let Some(&node) = registry.inline.get(&byte_pos) {
                    state.ensure_in_paragraph(writer)?;
                    render_node::render(node, true, writer)?;
                }
            }
            BLOCK_LEAF_SENTINEL => {
                if let Some(&node) = registry.block_leaf.get(&byte_pos) {
                    state.before_block_emit(writer)?;
                    render_node::render(node, true, writer)?;
                    state.after_block_emit();
                }
            }
            BLOCK_OPEN_SENTINEL => {
                if let Some(&kind) = registry.block_open.get(&byte_pos) {
                    state.before_block_emit(writer)?;
                    let node = AozoraNode::Container(Container { kind });
                    render_node::render(node, true, writer)?;
                    state.after_block_emit();
                }
            }
            BLOCK_CLOSE_SENTINEL => {
                if let Some(&kind) = registry.block_close.get(&byte_pos) {
                    state.before_block_emit(writer)?;
                    let node = AozoraNode::Container(Container { kind });
                    render_node::render(node, false, writer)?;
                    state.after_block_emit();
                }
            }
            '\n' => match bytes.get(pos + 1) {
                Some(&b'\n') => state.close_paragraph(writer)?,
                Some(_) if state.in_paragraph => writer.write_str("<br />\n")?,
                Some(_) | None => {}
            },
            _ => unreachable!("is_structural_char admitted only sentinels and \\n"),
        }
        cursor = pos + match_str.len();
    }

    if cursor < normalized.len() {
        state.ensure_in_paragraph(writer)?;
        escape_text_chunk(&normalized[cursor..], writer)?;
    }
    state.close_paragraph(writer)?;
    Ok(())
}

/// Block-level walker state — mirrors the owned renderer's state
/// machine 1:1 so the emitted bytes match. See
/// `aozora_parser::html::RenderState` for the design rationale.
#[derive(Debug, Default)]
struct RenderState {
    in_paragraph: bool,
    pending_block_separator: bool,
}

impl RenderState {
    fn flush_pending_separator<W: fmt::Write>(&mut self, out: &mut W) -> fmt::Result {
        if self.pending_block_separator {
            out.write_char('\n')?;
            self.pending_block_separator = false;
        }
        Ok(())
    }

    fn ensure_in_paragraph<W: fmt::Write>(&mut self, out: &mut W) -> fmt::Result {
        if !self.in_paragraph {
            self.flush_pending_separator(out)?;
            out.write_str("<p>")?;
            self.in_paragraph = true;
        }
        Ok(())
    }

    fn close_paragraph<W: fmt::Write>(&mut self, out: &mut W) -> fmt::Result {
        if self.in_paragraph {
            out.write_str("</p>\n")?;
            self.in_paragraph = false;
            self.pending_block_separator = false;
        }
        Ok(())
    }

    fn before_block_emit<W: fmt::Write>(&mut self, out: &mut W) -> fmt::Result {
        self.close_paragraph(out)?;
        self.flush_pending_separator(out)
    }

    fn after_block_emit(&mut self) {
        self.pending_block_separator = true;
    }
}

#[inline]
fn is_structural_char(c: char) -> bool {
    matches!(
        c,
        INLINE_SENTINEL | BLOCK_LEAF_SENTINEL | BLOCK_OPEN_SENTINEL | BLOCK_CLOSE_SENTINEL | '\n'
    )
}

fn escape_text_chunk<W: fmt::Write>(chunk: &str, out: &mut W) -> fmt::Result {
    let mut cursor = 0usize;
    for (pos, m) in chunk.match_indices(is_html_unsafe_char) {
        out.write_str(&chunk[cursor..pos])?;
        let ch = m.chars().next().expect("non-empty match");
        out.write_str(html_entity_for(ch))?;
        cursor = pos + m.len();
    }
    out.write_str(&chunk[cursor..])
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
        // Matches the outer renderer's `&#39;` (decimal). Per-node
        // renderer uses the hex form `&#x27;` for content inside Aozora
        // node payloads — that's a long-standing quirk of the legacy
        // renderer (aozora_parser::html uses decimal for inter-sentinel
        // chunks; aozora_parser::aozora::html uses hex for node payload).
        // Mirroring both forms keeps byte-identical equivalence.
        '\'' => "&#39;",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aozora_syntax::borrowed::Arena;
    use pretty_assertions::assert_eq;

    fn render(src: &str) -> String {
        let arena = Arena::new();
        let out = aozora_lex::lex_into_arena(src, &arena);
        render_to_string(&out)
    }

    #[test]
    fn plain_paragraph_wraps_in_p() {
        assert_eq!(render("Hello."), "<p>Hello.</p>\n");
    }

    #[test]
    fn ruby_emits_semantic_form() {
        let html = render("｜青梅《おうめ》");
        assert!(html.contains("<ruby>青梅"), "missing ruby tag: {html}");
        assert!(html.contains("<rt>おうめ"), "missing rt tag: {html}");
    }

    #[test]
    fn page_break_inside_text_emits_div() {
        let html = render("前\n\n［＃改ページ］\n\n後");
        assert!(html.contains(r#"<div class="afm-page-break"></div>"#));
        assert!(!html.contains("［＃"), "［＃ leaked: {html}");
    }

    #[test]
    fn paired_container_open_close_renders_div_pair() {
        let html = render("［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］");
        assert!(html.contains("afm-container-indent afm-container-indent-2"));
        assert!(html.contains("</div>"));
    }

    #[test]
    fn newline_inside_paragraph_emits_br() {
        let html = render("a\nb");
        assert!(html.contains("a<br />\nb"));
    }

    #[test]
    fn double_newline_closes_paragraph() {
        let html = render("a\n\nb");
        assert!(html.contains("<p>a</p>\n"));
        assert!(html.contains("<p>b</p>\n"));
    }

    #[test]
    fn html_unsafe_chars_in_plain_text_are_escaped() {
        let html = render("a<b>&\"'");
        assert!(html.contains("a&lt;b&gt;&amp;&quot;&#39;"));
    }

    #[test]
    fn empty_input_emits_empty_string() {
        assert_eq!(render(""), "");
    }
}
