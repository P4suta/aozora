//! Borrowed-AST HTML rendering.
//!
//! Consumes a [`LexOutput`] directly and emits semantic HTML5.
//!
//! # Algorithm
//!
//! Single forward `match_indices` sweep over the lexer's normalized
//! text, capturing every PUA sentinel + `\n` in O(n). Plain text
//! between matches flows through the bulk-copy escape pass; sentinels
//! dispatch into [`crate::render_node::render`] via the borrowed
//! registry's `EytzingerMap::get` lookup.
//!
//! Block structure mirrors the legacy walker: a two-state machine
//! `RenderState::ensure_in_paragraph` / `RenderState::close_paragraph`
//! emits `<p>` / `</p>` symmetrically; standalone block nodes (and
//! container open/close events) flush the current paragraph first.

use core::fmt;

use aozora_pipeline::LexOutput;
use aozora_syntax::borrowed::{Node, NodeRef};
use aozora_syntax::{Container, ContainerKind};
use memchr::{memchr_iter, memchr3_iter};

use crate::render_node;
use crate::walk::{SentinelKind, WalkSink, walk};

/// Render a `LexOutput` into a fresh `String`.
///
/// Allocates roughly `2 × normalized.len()` upfront. For streaming
/// consumers prefer [`render_into`] to avoid the intermediate `String`.
///
/// # Panics
///
/// Does not panic in normal use: `String` cannot fail as a
/// [`fmt::Write`] sink. The internal `expect` covers the trivially
/// unreachable case.
#[must_use]
pub fn render_to_string(out: &LexOutput<'_>) -> String {
    let mut s = String::with_capacity(out.normalized.len().saturating_mul(2));
    render_into(out, &mut s).expect("writing to String never fails");
    s
}

/// Render a `LexOutput` into the given writer.
///
/// # Errors
///
/// Propagates write errors from `writer`.
///
/// # Panics
///
/// Panics if the normalized text exceeds `u32::MAX` bytes — inherited
/// from the lexer's `Span` width contract; in practice unreachable
/// (the sanitize stage already gates on this bound).
pub fn render_into<W: fmt::Write>(out: &LexOutput<'_>, writer: &mut W) -> fmt::Result {
    let mut sink = HtmlSink {
        out: writer,
        state: RenderState::default(),
    };
    walk(out, &mut sink)
}

/// [`WalkSink`] that emits semantic HTML5: HTML-escapes every plain run
/// and drives the block-level [`RenderState`] (`<p>` / `<br />` /
/// container brackets) from the walk's sentinel and newline events.
struct HtmlSink<'w, W: fmt::Write> {
    out: &'w mut W,
    state: RenderState,
}

impl<W: fmt::Write> WalkSink for HtmlSink<'_, W> {
    // HTML output treats `\n` as structural (paragraph / line break).
    const WANTS_NEWLINES: bool = true;

    fn on_text(&mut self, text: &str) -> fmt::Result {
        self.state.ensure_in_paragraph(self.out)?;
        escape_text_chunk(text, self.out)
    }

    fn on_newline(&mut self, next: Option<u8>) -> fmt::Result {
        match next {
            // A blank line closes the current paragraph.
            Some(b'\n') => self.state.close_paragraph(self.out),
            // A lone newline inside a paragraph is a line break.
            Some(_) if self.state.in_paragraph => self.out.write_str("<br />\n"),
            // A newline outside a paragraph (e.g. between blocks) is dropped.
            Some(_) | None => Ok(()),
        }
    }

    fn on_node(&mut self, kind: SentinelKind, node: NodeRef<'_>) -> fmt::Result {
        match (kind, node) {
            (SentinelKind::Inline, NodeRef::Inline(node)) => {
                self.state.ensure_in_paragraph(self.out)?;
                render_node::render(node, true, self.out)
            }
            (SentinelKind::BlockLeaf, NodeRef::BlockLeaf(node)) => {
                self.state.before_block_emit(self.out)?;
                render_node::render(node, true, self.out)?;
                self.state.after_block_emit();
                Ok(())
            }
            (SentinelKind::BlockOpen, NodeRef::BlockOpen(kind)) => {
                self.state.open_container(kind, self.out)
            }
            (SentinelKind::BlockClose, NodeRef::BlockClose(kind)) => {
                self.state.close_container(kind, self.out)
            }
            // Sentinel without a matching registry entry: best-effort
            // skip, mirroring the legacy walker.
            _ => Ok(()),
        }
    }

    fn finish(&mut self) -> fmt::Result {
        self.state.close_paragraph(self.out)
    }
}

/// Block-level walker state. Tracks paragraph and block-separator
/// boundaries so consecutive inline runs collapse into one paragraph
/// and adjacent block-leaf nodes get the right inter-block whitespace.
#[derive(Debug, Default)]
struct RenderState {
    in_paragraph: bool,
    pending_block_separator: bool,
    /// Inside a phrasing-content container (a heading): its `<hN>` is the
    /// inline context, so [`Self::ensure_in_paragraph`] suppresses `<p>`.
    in_heading: bool,
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
        // Phrasing content inside a heading renders directly under the `<hN>`;
        // the heading element is the inline context, so no `<p>` is opened.
        if self.in_heading {
            return Ok(());
        }
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

    /// Emit a container's opening tag, honouring its content model. An inline
    /// container (傍点 / 傍線 range, bare-range 太字 / 斜体) stays in the
    /// current paragraph; a phrasing-content container (a heading) flushes the
    /// paragraph and then holds its content inline under the `<hN>`
    /// (`in_heading`); every other block container flushes and brackets its
    /// content as block paragraphs.
    fn open_container<W: fmt::Write>(&mut self, kind: ContainerKind, out: &mut W) -> fmt::Result {
        let node = Node::Container(Container { kind });
        if kind.is_inline() {
            self.ensure_in_paragraph(out)?;
            return render_node::render(node, true, out);
        }
        self.before_block_emit(out)?;
        render_node::render(node, true, out)?;
        if kind.content_is_phrasing() {
            self.in_heading = true;
        } else {
            self.after_block_emit();
        }
        Ok(())
    }

    /// Emit a container's closing tag — the mirror of [`Self::open_container`].
    fn close_container<W: fmt::Write>(&mut self, kind: ContainerKind, out: &mut W) -> fmt::Result {
        let node = Node::Container(Container { kind });
        if kind.is_inline() {
            self.ensure_in_paragraph(out)?;
            return render_node::render(node, false, out);
        }
        if kind.content_is_phrasing() {
            self.in_heading = false;
        } else {
            self.before_block_emit(out)?;
        }
        render_node::render(node, false, out)?;
        self.after_block_emit();
        Ok(())
    }
}

/// HTML-escape a plain-text chunk (the bytes between two structural
/// matches in [`render_into`]).
///
/// The five HTML-unsafe ASCII characters (`< > & " '`) are rare in
/// Japanese-text-heavy corpora — most chunks contain none. Two
/// `memchr` passes (`memchr3` for `< > &` then `memchr` for `"`)
/// fast-skip those clean chunks at memory-bandwidth speed; only when
/// at least one needle hits do we fall through to a byte loop that
/// merges the candidate positions and emits the escapes in document
/// order. Single-quote `'` (0x27) is folded into the same byte loop
/// because it has no `memchr_iter` partner — three needle scans are
/// enough to cover the rare cases without paying for a 5-needle
/// general scan, which `memchr` doesn't expose.
fn escape_text_chunk<W: fmt::Write>(chunk: &str, out: &mut W) -> fmt::Result {
    let bytes = chunk.as_bytes();

    // Fast-reject: no HTML-unsafe byte → bulk write the whole chunk.
    let mut iter_lt_gt_amp = memchr3_iter(b'<', b'>', b'&', bytes);
    let first_lt_gt_amp = iter_lt_gt_amp.next();
    let mut iter_quote = memchr_iter(b'"', bytes);
    let first_quote = iter_quote.next();
    let mut iter_apos = memchr_iter(b'\'', bytes);
    let first_apos = iter_apos.next();

    if first_lt_gt_amp.is_none() && first_quote.is_none() && first_apos.is_none() {
        return out.write_str(chunk);
    }

    // Slow path: merge the three iterators in document order.
    // Re-derive the iterators so we can use the post-`first_*`
    // peekable state cleanly. Cost is one duplicate memchr scan;
    // negligible because this branch only runs when the chunk
    // actually has unsafe bytes (rare on Japanese prose).
    let mut cursor = 0usize;
    let mut next_lt_gt_amp = first_lt_gt_amp;
    let mut next_quote = first_quote;
    let mut next_apos = first_apos;

    loop {
        // Pick the smallest of the three pending positions.
        let pos = [next_lt_gt_amp, next_quote, next_apos]
            .into_iter()
            .flatten()
            .min();
        let Some(pos) = pos else { break };

        if cursor < pos {
            out.write_str(&chunk[cursor..pos])?;
        }
        let entity = match bytes[pos] {
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'&' => "&amp;",
            b'"' => "&quot;",
            // Hex form `&#x27;` matches the canonical entity used by
            // `render_node::escape_text` so the streaming and the
            // per-node renderers produce byte-identical output for
            // every text chunk. Pinned by `tests/byte_identical_html.rs`.
            b'\'' => "&#x27;",
            // The match is exhaustive over the bytes the three
            // memchr scans yield — if we ever hit this branch,
            // either memchr returned a position outside its needle
            // set (impossible) or an iterator was advanced
            // incorrectly. Either way an unreachable! is the only
            // honest reaction.
            _ => unreachable!("escape iterator yielded non-needle byte"),
        };
        out.write_str(entity)?;
        cursor = pos + 1;

        // Advance whichever iterator just produced this position.
        if next_lt_gt_amp == Some(pos) {
            next_lt_gt_amp = iter_lt_gt_amp.next();
        }
        if next_quote == Some(pos) {
            next_quote = iter_quote.next();
        }
        if next_apos == Some(pos) {
            next_apos = iter_apos.next();
        }
    }
    out.write_str(&chunk[cursor..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use aozora_syntax::borrowed::Arena;
    use pretty_assertions::assert_eq;

    fn render(src: &str) -> String {
        let arena = Arena::new();
        let out = aozora_pipeline::lex(src, &arena);
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
        assert!(html.contains(r#"<div class="aozora-page-break"></div>"#));
        assert!(!html.contains("［＃"), "［＃ leaked: {html}");
    }

    #[test]
    fn paired_container_open_close_renders_div_pair() {
        let html = render("［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］");
        assert!(html.contains("aozora-container-indent aozora-container-indent-2"));
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
        assert!(
            html.contains("a&lt;b&gt;&amp;&quot;&#x27;"),
            "expected byte-identical entities (incl. `&#x27;` for apostrophe), got: {html}",
        );
    }

    #[test]
    fn empty_input_emits_empty_string() {
        assert_eq!(render(""), "");
    }

    #[test]
    fn inline_container_stays_inside_paragraph() {
        // A bare 太字 range is inline: it must open / close *inside* the
        // surrounding `<p>`, not flush it.
        let html = render("前［＃太字］中［＃太字終わり］後");
        assert_eq!(
            html, "<p>前<b class=\"aozora-bold\">中</b>後</p>\n",
            "inline container must stay within the paragraph",
        );
    }

    #[test]
    fn block_container_flushes_paragraph_then_wraps_body() {
        let html = render("前文\n\n［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］");
        assert!(html.contains("<p>前文</p>\n"), "leading paragraph: {html}");
        assert!(
            html.contains(
                "<div class=\"aozora-container aozora-container-indent aozora-container-indent-2\""
            ),
            "indent container open: {html}"
        );
        assert!(
            html.contains("<p>本文</p>"),
            "wrapped body paragraph: {html}"
        );
        assert!(html.contains("</div>"), "container close: {html}");
    }

    #[test]
    fn heading_container_holds_content_inline_without_inner_paragraph() {
        // A heading container's content is phrasing: it renders directly
        // inside the `<hN>` with no inner `<p>` (the `in_heading` flag).
        let html = render("［＃ここから大見出し］\n章題\n［＃ここで大見出し終わり］");
        assert!(
            html.contains("<h1 class=\"aozora-heading aozora-heading-large\">章題</h1>"),
            "heading must hold text inline without a <p>: {html}"
        );
        assert!(
            !html.contains("<h1 class=\"aozora-heading aozora-heading-large\"><p>"),
            "heading must not wrap content in <p>: {html}"
        );
    }

    #[test]
    fn section_break_block_flushes_surrounding_paragraphs() {
        let html = render("前\n\n［＃改丁］\n\n後");
        assert!(
            html.contains("<p>前</p>\n"),
            "paragraph before break: {html}"
        );
        assert!(
            html.contains("<div class=\"aozora-section-break aozora-section-break-kaicho\"></div>"),
            "section break div: {html}"
        );
        assert!(
            html.contains("<p>後</p>\n"),
            "paragraph after break: {html}"
        );
    }

    #[test]
    fn single_trailing_newline_emits_no_break_outside_paragraph() {
        // A lone trailing `\n` with nothing after it (and no open
        // paragraph) must not emit a stray `<br />`.
        let html = render("a\n");
        assert_eq!(html, "<p>a</p>\n", "trailing newline must not add <br />");
    }

    #[test]
    fn quote_and_apostrophe_chunk_take_the_slow_escape_path() {
        // Mixing `"` and `'` with `<`/`>`/`&` exercises the three-iterator
        // merge in `escape_text_chunk`.
        let html = render(r#"x"y'z<&>"#);
        assert_eq!(
            html, "<p>x&quot;y&#x27;z&lt;&amp;&gt;</p>\n",
            "all five unsafe chars must escape in document order",
        );
    }

    #[test]
    fn apostrophe_only_chunk_escapes_via_byte_loop() {
        // `'` has no memchr partner; a chunk with only `'` still escapes.
        let html = render("it's");
        assert_eq!(html, "<p>it&#x27;s</p>\n", "lone apostrophe must escape");
    }
}
