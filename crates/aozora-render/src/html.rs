//! Block-level HTML render state + text escaper.
//!
//! The shared, AST-free machinery the owned HTML renderer
//! ([`crate::html_owned`]) drives: the two-state paragraph machine
//! ([`RenderState`]) that emits `<p>` / `<br />` / container brackets from the
//! walk's sentinel + newline events, and the bulk-copy [`escape_text_chunk`]
//! pass over plain runs. Container open/close tags route through the
//! lifetime-free [`render_node::render_container`], so the byte spelling stays
//! single-source.

use core::fmt;

use aozora_syntax::{Container, RegionFormat};
use memchr::{memchr_iter, memchr3_iter};

use crate::render_node;

/// Block-level walker state. Tracks paragraph and block-separator boundaries so
/// consecutive inline runs collapse into one paragraph and adjacent block-leaf
/// nodes get the right inter-block whitespace.
#[derive(Debug, Default)]
pub(crate) struct RenderState {
    pub(crate) in_paragraph: bool,
    pending_block_separator: bool,
    /// Inside a phrasing-content container (a heading): its `<hN>` is the inline
    /// context, so [`Self::ensure_in_paragraph`] suppresses `<p>`.
    in_heading: bool,
    /// In-flight container opens. The close marker reads the matched open
    /// [`RegionFormat`] (open-authoritative).
    open_stack: Vec<RegionFormat>,
}

impl RenderState {
    fn flush_pending_separator<W: fmt::Write>(&mut self, out: &mut W) -> fmt::Result {
        if self.pending_block_separator {
            out.write_char('\n')?;
            self.pending_block_separator = false;
        }
        Ok(())
    }

    pub(crate) fn ensure_in_paragraph<W: fmt::Write>(&mut self, out: &mut W) -> fmt::Result {
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

    pub(crate) fn close_paragraph<W: fmt::Write>(&mut self, out: &mut W) -> fmt::Result {
        if self.in_paragraph {
            out.write_str("</p>\n")?;
            self.in_paragraph = false;
            self.pending_block_separator = false;
        }
        Ok(())
    }

    pub(crate) fn before_block_emit<W: fmt::Write>(&mut self, out: &mut W) -> fmt::Result {
        self.close_paragraph(out)?;
        self.flush_pending_separator(out)
    }

    pub(crate) fn after_block_emit(&mut self) {
        self.pending_block_separator = true;
    }

    /// Emit a container's opening tag, honouring its content model. An inline
    /// container stays in the current paragraph; a phrasing-content container (a
    /// heading) flushes the paragraph and holds its content inline under the
    /// `<hN>` (`in_heading`); every other block container flushes and brackets
    /// its content as block paragraphs.
    pub(crate) fn open_container<W: fmt::Write>(
        &mut self,
        kind: RegionFormat,
        out: &mut W,
    ) -> fmt::Result {
        self.open_stack.push(kind);
        let container = Container { kind };
        if kind.is_inline() {
            self.ensure_in_paragraph(out)?;
            return render_node::render_container(container, true, out);
        }
        self.before_block_emit(out)?;
        render_node::render_container(container, true, out)?;
        if kind.content_is_phrasing() {
            self.in_heading = true;
        } else {
            self.after_block_emit();
        }
        Ok(())
    }

    /// Emit a container's closing tag — the mirror of [`Self::open_container`],
    /// reconstructed from the matched open [`RegionFormat`] popped off the stack
    /// (open-authoritative). A degraded empty stack best-effort skips.
    pub(crate) fn close_container<W: fmt::Write>(&mut self, out: &mut W) -> fmt::Result {
        let Some(kind) = self.open_stack.pop() else {
            return Ok(());
        };
        let container = Container { kind };
        if kind.is_inline() {
            self.ensure_in_paragraph(out)?;
            return render_node::render_container(container, false, out);
        }
        if kind.content_is_phrasing() {
            self.in_heading = false;
        } else {
            self.before_block_emit(out)?;
        }
        render_node::render_container(container, false, out)?;
        self.after_block_emit();
        Ok(())
    }

    /// Close every container left open at end-of-input. A source may open a
    /// region (`［＃ここから字下げ］`) without a matching close; the AST and
    /// `to_source` preserve that imbalance verbatim, but `to_html` must still
    /// emit valid, balanced markup — the unclosed region renders as extending
    /// to the end of the document. Mirrors the per-marker [`Self::close_container`].
    pub(crate) fn drain_open_containers<W: fmt::Write>(&mut self, out: &mut W) -> fmt::Result {
        while !self.open_stack.is_empty() {
            self.close_container(out)?;
        }
        Ok(())
    }
}

/// HTML-escape a plain-text chunk (the bytes between two structural matches in
/// the streaming walk).
///
/// The five HTML-unsafe ASCII characters (`< > & " '`) are rare in
/// Japanese-text-heavy corpora — most chunks contain none. Two `memchr` passes
/// (`memchr3` for `< > &` then `memchr` for `"`) fast-skip those clean chunks at
/// memory-bandwidth speed; only when at least one needle hits do we fall through
/// to a byte loop that merges the candidate positions and emits the escapes in
/// document order.
pub(crate) fn escape_text_chunk<W: fmt::Write>(chunk: &str, out: &mut W) -> fmt::Result {
    let bytes = chunk.as_bytes();

    let mut iter_lt_gt_amp = memchr3_iter(b'<', b'>', b'&', bytes);
    let first_lt_gt_amp = iter_lt_gt_amp.next();
    let mut iter_quote = memchr_iter(b'"', bytes);
    let first_quote = iter_quote.next();
    let mut iter_apos = memchr_iter(b'\'', bytes);
    let first_apos = iter_apos.next();

    if first_lt_gt_amp.is_none() && first_quote.is_none() && first_apos.is_none() {
        return out.write_str(chunk);
    }

    let mut cursor = 0usize;
    let mut next_lt_gt_amp = first_lt_gt_amp;
    let mut next_quote = first_quote;
    let mut next_apos = first_apos;

    loop {
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
            // Hex form `&#x27;` matches `render_node::escape_text` so the
            // streaming and per-node renderers produce byte-identical output.
            b'\'' => "&#x27;",
            _ => unreachable!("escape iterator yielded non-needle byte"),
        };
        out.write_str(entity)?;
        cursor = pos + 1;

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
    use crate::render_html_owned;
    use pretty_assertions::assert_eq;

    fn render(src: &str) -> String {
        render_html_owned(&aozora_pipeline::lex(src))
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
        let html = render("前［＃太字］中［＃太字終わり］後");
        assert_eq!(
            html, "<p>前<b class=\"aozora-futoji\">中</b>後</p>\n",
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
        let html = render("a\n");
        assert_eq!(html, "<p>a</p>\n", "trailing newline must not add <br />");
    }

    #[test]
    fn quote_and_apostrophe_chunk_take_the_slow_escape_path() {
        let html = render(r#"x"y'z<&>"#);
        assert_eq!(
            html, "<p>x&quot;y&#x27;z&lt;&amp;&gt;</p>\n",
            "all five unsafe chars must escape in document order",
        );
    }

    #[test]
    fn apostrophe_only_chunk_escapes_via_byte_loop() {
        let html = render("it's");
        assert_eq!(html, "<p>it&#x27;s</p>\n", "lone apostrophe must escape");
    }

    #[test]
    fn referenced_contiguous_forward_styles_referent_once() {
        // #333: the non-adjacent referent 青空 is now styled in place (a
        // `Detached` decoration spliced into the plain run), while the bracket
        // stays `Referenced` and renders nothing. 青空 still appears exactly
        // once — the styling is added, the #228 no-double-render invariant holds.
        let html = render("青空の下を歩く［＃「青空」に傍点］");
        assert_eq!(
            html,
            "<p><em class=\"aozora-bouten aozora-bouten-goma aozora-bouten-right\">青空</em>の下を歩く</p>\n"
        );
        assert_eq!(html.matches("青空").count(), 1, "青空 must not duplicate");
        assert!(html.contains("<em"), "referent now styled: {html}");
    }

    #[test]
    fn referenced_ruby_base_forward_styles_base_once() {
        // #384: the forward target 我 is a ruby base, so it cannot be pulled into
        // a plain forward leaf; the lowering pass instead decorates the ruby's
        // base (render-only `base_emphasis`). The bracket stays `Referenced` and
        // renders nothing, so 我 appears exactly once — now styled inside the
        // `<ruby>`, before the `<rt>` — and the #228 no-double-render invariant
        // still holds.
        let html = render("我《われ》の名は［＃「我」に傍点］");
        assert_eq!(
            html,
            "<p><ruby><em class=\"aozora-bouten aozora-bouten-goma aozora-bouten-right\">我</em><rp>(</rp><rt>われ</rt><rp>)</rp></ruby>の名は</p>\n"
        );
        assert_eq!(html.matches("我").count(), 1, "我 must not duplicate");
        assert!(html.contains("<em"), "ruby base now styled (#384): {html}");
    }

    #[test]
    fn reclaimed_adjacent_forward_still_renders_emphasis() {
        let html = render("青空［＃「青空」に傍点］を見上げる。");
        assert_eq!(
            html,
            "<p><em class=\"aozora-bouten aozora-bouten-goma aozora-bouten-right\">青空</em>を見上げる。</p>\n"
        );
    }
}
