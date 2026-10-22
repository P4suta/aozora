#![expect(
    clippy::expect_used,
    reason = "the spelling table is validated before lookup"
)]

use core::fmt::{self, Write};
use std::collections::VecDeque;

use crate::spec::roman_slug;
use crate::syntax::{
    BlockStyles, BoutenPosition, Container, HeadingKind, HeadingStyle, IndentBlock, IndentLayout,
    LineFormat, RegionFormat,
};
use memchr::{memchr_iter, memchr3_iter};

use crate::render::classes;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineScope {
    Region(RegionFormat),
    Warichu,
}

fn render_inline_scope<W: Write>(scope: InlineScope, entering: bool, out: &mut W) -> fmt::Result {
    match scope {
        InlineScope::Region(kind) => render_container(Container { kind }, entering, out),
        InlineScope::Warichu if entering => out.write_str(r#"<span class="aozora-warichu">"#),
        InlineScope::Warichu => out.write_str("</span>"),
    }
}

#[derive(Debug, Default)]
pub(crate) struct RenderState {
    pub(crate) in_paragraph: bool,
    pending_block_separator: bool,
    in_heading: bool,
    open_stack: Vec<RegionFormat>,
    reopen_after_para: VecDeque<RegionFormat>,
    inline_stack: Vec<InlineScope>,
}

impl RenderState {
    fn flush_pending_separator<W: Write>(&mut self, out: &mut W) -> fmt::Result {
        if self.pending_block_separator {
            out.write_char('\n')?;
            self.pending_block_separator = false;
        }
        Ok(())
    }

    pub(crate) fn ensure_in_paragraph<W: Write>(&mut self, out: &mut W) -> fmt::Result {
        if self.in_heading {
            return Ok(());
        }
        if !self.in_paragraph {
            self.flush_pending_separator(out)?;
            out.write_str("<p>")?;
            self.in_paragraph = true;
            while let Some(kind) = self.reopen_after_para.pop_back() {
                render_container(Container { kind }, true, out)?;
                self.open_stack.push(kind);
                self.inline_stack.push(InlineScope::Region(kind));
            }
        }
        Ok(())
    }

    pub(crate) fn close_paragraph<W: Write>(&mut self, out: &mut W) -> fmt::Result {
        if self.in_paragraph {
            while let Some(scope) = self.inline_stack.pop() {
                render_inline_scope(scope, false, out)?;
                if let InlineScope::Region(kind) = scope {
                    let open = self.open_stack.pop();
                    debug_assert_eq!(
                        open,
                        Some(kind),
                        "inline and container region stacks diverged"
                    );
                    self.reopen_after_para.push_back(kind);
                }
            }
            out.write_str("</p>\n")?;
            self.in_paragraph = false;
            self.pending_block_separator = false;
        } else {
            self.drain_open_warichu(out)?;
        }
        Ok(())
    }

    pub(crate) fn before_block_emit<W: Write>(&mut self, out: &mut W) -> fmt::Result {
        self.close_paragraph(out)?;
        self.flush_pending_separator(out)
    }

    pub(crate) fn after_block_emit(&mut self) {
        self.pending_block_separator = true;
    }

    pub(crate) fn open_container<W: Write>(
        &mut self,
        kind: RegionFormat,
        out: &mut W,
    ) -> fmt::Result {
        let container = Container { kind };
        if kind.is_inline() {
            self.ensure_in_paragraph(out)?;
            render_container(container, true, out)?;
            self.open_stack.push(kind);
            self.inline_stack.push(InlineScope::Region(kind));
            return Ok(());
        }
        self.before_block_emit(out)?;
        self.open_stack.push(kind);
        render_container(container, true, out)?;
        if kind.content_is_phrasing() {
            self.in_heading = true;
        } else {
            self.after_block_emit();
        }
        Ok(())
    }

    pub(crate) fn close_container<W: Write>(
        &mut self,
        closing_inline: bool,
        out: &mut W,
    ) -> fmt::Result {
        if closing_inline && self.reopen_after_para.pop_front().is_some() {
            return Ok(());
        }
        let Some(kind) = self.open_stack.pop() else {
            return Ok(());
        };
        let container = Container { kind };
        if kind.is_inline() {
            self.ensure_in_paragraph(out)?;
            if let Some(index) = self
                .inline_stack
                .iter()
                .rposition(|scope| *scope == InlineScope::Region(kind))
            {
                self.close_inline_scope_at(index, out)?;
            }
            return Ok(());
        }
        if kind.content_is_phrasing() {
            self.drain_open_warichu(out)?;
            self.in_heading = false;
        } else {
            self.before_block_emit(out)?;
        }
        render_container(container, false, out)?;
        self.after_block_emit();
        Ok(())
    }

    pub(crate) fn open_warichu<W: Write>(&mut self, out: &mut W) -> fmt::Result {
        render_inline_scope(InlineScope::Warichu, true, out)?;
        self.inline_stack.push(InlineScope::Warichu);
        Ok(())
    }

    pub(crate) fn close_warichu<W: Write>(&mut self, out: &mut W) -> fmt::Result {
        if let Some(index) = self
            .inline_stack
            .iter()
            .rposition(|scope| matches!(scope, InlineScope::Warichu))
        {
            self.close_inline_scope_at(index, out)?;
        }
        Ok(())
    }

    pub(crate) fn drain_open_warichu<W: Write>(&mut self, out: &mut W) -> fmt::Result {
        let open_count = self
            .inline_stack
            .iter()
            .filter(|scope| matches!(scope, InlineScope::Warichu))
            .count();
        for _ in 0..open_count {
            let Some(index) = self
                .inline_stack
                .iter()
                .rposition(|scope| matches!(scope, InlineScope::Warichu))
            else {
                break;
            };
            self.close_inline_scope_at(index, out)?;
        }
        Ok(())
    }

    fn close_inline_scope_at<W: Write>(&mut self, index: usize, out: &mut W) -> fmt::Result {
        let mut reopen = Vec::new();
        while self.inline_stack.len() > index.saturating_add(1) {
            let scope = self.inline_stack.pop().expect("scope above target");
            render_inline_scope(scope, false, out)?;
            reopen.push(scope);
        }
        let target = self.inline_stack.pop().expect("target scope");
        render_inline_scope(target, false, out)?;
        for scope in reopen.into_iter().rev() {
            render_inline_scope(scope, true, out)?;
            self.inline_stack.push(scope);
        }
        Ok(())
    }

    pub(crate) fn finish<W: Write>(&mut self, out: &mut W) -> fmt::Result {
        self.close_paragraph(out)?;
        self.reopen_after_para.clear();
        let mut closed = false;
        while let Some(kind) = self.open_stack.pop() {
            render_container(Container { kind }, false, out)?;
            closed = true;
        }
        self.inline_stack.clear();
        if closed {
            out.write_char('\n')?;
        }
        self.in_heading = false;
        self.pending_block_separator = false;
        Ok(())
    }
}

pub(crate) fn escape_text_chunk<W: Write>(chunk: &str, out: &mut W) -> fmt::Result {
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

        out.write_str(&chunk[cursor..pos])?;
        let entity = match bytes[pos] {
            b'<' => {
                next_lt_gt_amp = iter_lt_gt_amp.next();
                "&lt;"
            }
            b'>' => {
                next_lt_gt_amp = iter_lt_gt_amp.next();
                "&gt;"
            }
            b'&' => {
                next_lt_gt_amp = iter_lt_gt_amp.next();
                "&amp;"
            }
            b'"' => {
                next_quote = iter_quote.next();
                "&quot;"
            }
            b'\'' => {
                next_apos = iter_apos.next();
                "&#x27;"
            }
            _ => unreachable!("escape iterator yielded non-needle byte"),
        };
        out.write_str(entity)?;
        cursor = pos.checked_add(1).expect("escape offset fits usize");
    }
    out.write_str(&chunk[cursor..])
}

pub(crate) fn render_container<W: Write>(
    c: Container,
    entering: bool,
    writer: &mut W,
) -> fmt::Result {
    if entering {
        render_container_open(c.kind, writer)
    } else {
        render_container_close(c.kind, writer)
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one match arm per RegionFormat — splitting would scatter the \
              1:1 kind→markup mapping that mirrors emit_container_open"
)]
fn render_container_open<W: Write>(kind: RegionFormat, writer: &mut W) -> fmt::Result {
    match kind {
        RegionFormat::Indent(IndentBlock {
            amount,
            wrap,
            center,
            layout,
            styles,
        }) => {
            let BlockStyles {
                gothic,
                horizontal,
                framed,
                font,
            } = styles;
            write!(
                writer,
                r#"<div class="aozora-container aozora-container-indent aozora-container-indent-{amount}"#,
            )?;
            if wrap.is_some() {
                writer.write_str(" aozora-container-wrap-indent")?;
            }
            if center {
                writer.write_str(" aozora-container-center")?;
            }
            match layout {
                IndentLayout::Kumi(_) => {
                    writer.write_str(" aozora-container-line-kumi")?;
                }
                IndentLayout::LineWidth(_) => {
                    writer.write_str(" aozora-container-line-width")?;
                }
                IndentLayout::None => {}
            }
            if gothic {
                writer.write_str(" aozora-container-goshikku")?;
            }
            if horizontal {
                writer.write_str(" aozora-container-yokogumi")?;
            }
            if framed {
                writer.write_str(" aozora-container-keigakomi")?;
            }
            if let Some(shift) = font {
                writer.write_str(if shift.larger() {
                    " aozora-container-font-larger"
                } else {
                    " aozora-container-font-smaller"
                })?;
            }
            write!(writer, r#"" data-amount="{amount}""#)?;
            if let Some(w) = wrap {
                write!(writer, r#" data-wrap="{w}""#)?;
            }
            match layout {
                IndentLayout::Kumi(kumi) => {
                    write!(
                        writer,
                        r#" data-kumi-lines="{}" data-kumi-width="{}""#,
                        kumi.lines, kumi.width
                    )?;
                }
                IndentLayout::LineWidth(width) => {
                    write!(writer, r#" data-width="{}""#, width.0)?;
                }
                IndentLayout::None => {}
            }
            if let Some(shift) = font {
                write!(writer, r#" data-steps="{}""#, shift.magnitude())?;
            }
            writer.write_str(">")
        }
        RegionFormat::AlignEnd { offset } => {
            write!(
                writer,
                r#"<div class="aozora-container aozora-container-align-end" data-offset="{offset}">"#,
            )
        }
        RegionFormat::LineWidth(width) => {
            write!(
                writer,
                r#"<div class="aozora-container aozora-container-line-width" data-width="{}">"#,
                width.0,
            )
        }
        RegionFormat::Framed(_) => {
            writer.write_str(r#"<div class="aozora-container aozora-container-keigakomi">"#)
        }
        RegionFormat::Warichu => {
            writer.write_str(r#"<div class="aozora-container aozora-container-warichu">"#)
        }
        RegionFormat::Bouten { kind, position } => {
            write!(
                writer,
                r#"<em class="aozora-bouten aozora-bouten-{kind} aozora-bouten-{pos}">"#,
                kind = classes::bouten_kind_slug(kind),
                pos = classes::bouten_position_slug(position),
            )
        }
        RegionFormat::Bold { padded: false } => writer.write_str(r#"<b class="aozora-futoji">"#),
        RegionFormat::Gothic { padded: false } => {
            writer.write_str(r#"<span class="aozora-goshikku">"#)
        }
        RegionFormat::Italic { padded: false } => writer.write_str(r#"<i class="aozora-shatai">"#),
        RegionFormat::Bold { padded: true } => {
            writer.write_str(r#"<div class="aozora-container aozora-container-futoji">"#)
        }
        RegionFormat::Gothic { padded: true } => {
            writer.write_str(r#"<div class="aozora-container aozora-container-goshikku">"#)
        }
        RegionFormat::Italic { padded: true } => {
            writer.write_str(r#"<div class="aozora-container aozora-container-shatai">"#)
        }
        RegionFormat::Columns(count) => write!(
            writer,
            r#"<div class="aozora-container aozora-container-columns" data-columns="{}">"#,
            count.0,
        ),
        RegionFormat::Table => {
            writer.write_str(r#"<div class="aozora-container aozora-container-table">"#)
        }
        RegionFormat::Horizontal => {
            writer.write_str(r#"<div class="aozora-container aozora-container-yokogumi">"#)
        }
        RegionFormat::FontSize(shift) => {
            let class = if shift.larger() {
                "aozora-container-font-larger"
            } else {
                "aozora-container-font-smaller"
            };
            write!(
                writer,
                r#"<div class="aozora-container {class}" data-steps="{}">"#,
                shift.magnitude(),
            )
        }
        RegionFormat::Heading { level, style, .. } => write_heading_open(level, style, writer),
        RegionFormat::SmallScript(BoutenPosition::Left) => {
            writer.write_str(r#"<span class="aozora-kogaki-left">"#)
        }
        RegionFormat::SmallScript(_) => writer.write_str(r#"<span class="aozora-kogaki-right">"#),
        RegionFormat::Caption { padded: false } => {
            writer.write_str(r#"<span class="aozora-caption">"#)
        }
        RegionFormat::Caption { padded: true } => {
            writer.write_str(r#"<div class="aozora-container aozora-caption">"#)
        }
    }
}

fn render_container_close<W: Write>(kind: RegionFormat, writer: &mut W) -> fmt::Result {
    let close = match kind {
        RegionFormat::Heading { level, style, .. } => {
            return write_heading_close(level, style, writer);
        }
        RegionFormat::Bouten { .. } => "</em>",
        RegionFormat::Bold { padded: false } => "</b>",
        RegionFormat::Italic { padded: false } => "</i>",
        RegionFormat::Gothic { padded: false }
        | RegionFormat::SmallScript(_)
        | RegionFormat::Caption { padded: false } => "</span>",
        RegionFormat::Bold { padded: true }
        | RegionFormat::Gothic { padded: true }
        | RegionFormat::Italic { padded: true }
        | RegionFormat::Caption { padded: true }
        | RegionFormat::Indent(_)
        | RegionFormat::Warichu
        | RegionFormat::Framed(_)
        | RegionFormat::AlignEnd { .. }
        | RegionFormat::LineWidth(_)
        | RegionFormat::Columns(_)
        | RegionFormat::Table
        | RegionFormat::Horizontal
        | RegionFormat::FontSize(_) => "</div>",
    };
    writer.write_str(close)
}

pub(crate) fn parse_sashie_dimensions(dims: &str) -> Option<(&str, &str)> {
    let (w, h) = dims.split_once('×')?;
    let w = w.strip_prefix('横')?;
    let h = h.strip_prefix('縦')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (digits(w) && digits(h)).then_some((w, h))
}

fn heading_tag(kind: HeadingKind, style: HeadingStyle) -> &'static str {
    if matches!(style, HeadingStyle::Window) {
        "div"
    } else {
        match kind {
            HeadingKind::Large => "h1",
            HeadingKind::Medium => "h2",
            HeadingKind::Small => "h3",
        }
    }
}

pub(crate) fn write_heading_open<W: Write>(
    kind: HeadingKind,
    style: HeadingStyle,
    writer: &mut W,
) -> fmt::Result {
    write!(
        writer,
        r#"<{tag} class="aozora-heading aozora-heading-{level_slug}"#,
        tag = heading_tag(kind, style),
        level_slug = classes::heading_level_slug(kind),
    )?;
    if let Some(modifier) = classes::heading_style_slug(style) {
        write!(writer, " aozora-heading-{modifier}")?;
    }
    writer.write_str(r#"">"#)
}

pub(crate) fn write_heading_close<W: Write>(
    kind: HeadingKind,
    style: HeadingStyle,
    writer: &mut W,
) -> fmt::Result {
    write!(writer, "</{}>", heading_tag(kind, style))
}

pub(crate) fn render_line<W: Write>(lf: LineFormat, writer: &mut W) -> fmt::Result {
    match lf {
        LineFormat::Indent {
            amount,
            end_offset: None,
        } => write!(
            writer,
            r#"<span class="aozora-indent aozora-indent-{amount}" data-amount="{amount}"></span>"#,
        ),
        LineFormat::Indent {
            amount,
            end_offset: Some(offset),
        } => write!(
            writer,
            r#"<span class="aozora-indent aozora-indent-{amount} aozora-align-end aozora-align-end-{offset}" data-amount="{amount}" data-offset="{offset}"></span>"#,
        ),
        LineFormat::AlignEnd { offset: 0 } => {
            writer.write_str(r#"<span class="aozora-align-end" data-offset="0"></span>"#)
        }
        LineFormat::AlignEnd { offset } => write!(
            writer,
            r#"<span class="aozora-align-end aozora-align-end-{offset}" data-offset="{offset}"></span>"#,
        ),
        LineFormat::Center { .. } => writer.write_str(r#"<span class="aozora-center"></span>"#),
        LineFormat::Gothic => writer.write_str(r#"<span class="aozora-line-goshikku"></span>"#),
        LineFormat::FontSizeAbsolute { size, bold } => {
            let slug = roman_slug(size.keyword()).unwrap_or("font-small");
            if bold {
                write!(
                    writer,
                    r#"<span class="aozora-line-{slug} aozora-line-futoji"></span>"#,
                )
            } else {
                write!(writer, r#"<span class="aozora-line-{slug}"></span>"#)
            }
        }
    }
}

pub(crate) fn escape_text<W: Write>(text: &str, writer: &mut W) -> fmt::Result {
    let mut cursor = 0;
    for (pos, m) in text.match_indices(HTML_UNSAFE_CHARS) {
        writer.write_str(&text[cursor..pos])?;
        let ch = m.as_bytes()[0] as char;
        writer.write_str(html_entity(ch))?;
        cursor = pos + m.len();
    }
    writer.write_str(&text[cursor..])
}

const HTML_UNSAFE_CHARS: &[char] = &['<', '>', '&', '"', '\''];

#[inline]
const fn html_entity(c: char) -> &'static str {
    match c {
        '<' => "&lt;",
        '>' => "&gt;",
        '&' => "&amp;",
        '"' => "&quot;",
        '\'' => "&#x27;",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use crate::render::render_html;
    use pretty_assertions::assert_eq;

    use super::{
        InlineScope, RenderState, escape_text, heading_tag, html_entity, parse_sashie_dimensions,
        render_container_close, render_container_open, render_line,
    };
    use crate::pipeline::lex;
    use crate::syntax::{
        BoutenKind, BoutenPosition, EnclosureKind, HeadingKind, HeadingStyle, LineFormat,
        LineWidth, RegionFormat,
    };
    use core::num::NonZeroU8;

    fn render(src: &str) -> String {
        render_html(&lex(src))
    }

    fn open_tag(kind: RegionFormat) -> String {
        let mut s = String::new();
        render_container_open(kind, &mut s).expect("render into String is infallible");
        s
    }

    fn close_tag(kind: RegionFormat) -> String {
        let mut s = String::new();
        render_container_close(kind, &mut s).expect("render into String is infallible");
        s
    }

    fn line_tag(lf: LineFormat) -> String {
        let mut s = String::new();
        render_line(lf, &mut s).expect("render into String is infallible");
        s
    }

    #[test]
    fn plain_paragraph_wraps_in_p() {
        assert_eq!(render("Hello."), "<p>Hello.</p>\n");
    }

    #[test]
    fn pending_block_separator_is_emitted_before_paragraph() {
        let mut state = RenderState::default();
        let mut out = String::new();
        state.after_block_emit();
        state
            .ensure_in_paragraph(&mut out)
            .expect("render into String is infallible");
        assert_eq!(out, "\n<p>");
    }

    #[test]
    fn closing_a_crossed_inline_scope_removes_only_its_target() {
        let mut state = RenderState::default();
        let mut out = String::new();
        let bold = RegionFormat::Bold { padded: false };
        state
            .open_container(bold, &mut out)
            .expect("render into String is infallible");
        state
            .open_warichu(&mut out)
            .expect("render into String is infallible");
        out.clear();

        state
            .close_inline_scope_at(0, &mut out)
            .expect("render into String is infallible");

        assert_eq!(out, "</span></b><span class=\"aozora-warichu\">");
        assert_eq!(state.inline_stack, vec![InlineScope::Warichu]);
    }

    #[test]
    fn warichu_wellformed_inline_pair_is_byte_identical() {
        let html = render("前［＃割り注］上等／下等［＃割り注終わり］後");
        assert_eq!(
            html,
            "<p>前<span class=\"aozora-warichu\">上等／下等</span>後</p>\n",
        );
        assert_eq!(
            html.matches("<span").count(),
            html.matches("</span>").count()
        );
    }

    #[test]
    fn warichu_block_open_inline_close_absorbs_stray_close() {
        let html = render("［＃ここから割り注］\n上等\n［＃割り注終わり］");
        assert_eq!(
            html.matches("<span").count(),
            html.matches("</span>").count(),
            "span tags must balance (no stray </span>): {html}",
        );
        assert!(
            !html.contains("</span>"),
            "no warichu span was opened, so no </span> should appear: {html}",
        );
        assert_eq!(html.matches("<div").count(), 1);
        assert_eq!(html.matches("</div>").count(), 1);
    }

    #[test]
    fn warichu_inline_open_block_close_drains_span() {
        let html = render("前［＃割り注］上等［＃ここで割り注終わり］後");
        assert_eq!(
            html.matches("<span").count(),
            html.matches("</span>").count(),
            "span tags must balance (open span must be drained): {html}",
        );
        assert!(
            html.contains(r#"<span class="aozora-warichu">"#),
            "the inline warichu span must still open: {html}",
        );
        assert!(
            html.contains("</span></p>"),
            "the span must close before the </p>, not straddle it: {html}",
        );
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
    fn unclosed_block_container_closes_at_eof() {
        let html = render("［＃ここから2字下げ］\n本文");
        assert_eq!(html.matches("<div").count(), html.matches("</div>").count());
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

    fn bold_never_straddles_p_close(html: &str) -> bool {
        let mut cursor = 0;
        while let Some(rel) = html[cursor..].find("</p>") {
            let end = cursor + rel;
            let prefix = &html[..end];
            if prefix.matches("<b").count() != prefix.matches("</b>").count() {
                return false;
            }
            cursor = end + "</p>".len();
        }
        true
    }

    #[test]
    fn unclosed_bold_across_paragraph_break_never_straddles_p() {
        let html = render("前［＃太字］中\n\n後");
        assert_eq!(
            html.matches("<b").count(),
            html.matches("</b>").count(),
            "bold must be globally balanced: {html}",
        );
        assert!(
            bold_never_straddles_p_close(&html),
            "no open <b> may straddle </p>: {html}",
        );
        assert!(
            !html.contains("<b class=\"aozora-futoji\">中</p>"),
            "the </b> must precede </p>, not straddle it: {html}",
        );
        assert!(
            html.contains("</b></p>"),
            "bold closes before the paragraph boundary: {html}",
        );
    }

    #[test]
    fn unclosed_bold_reaching_eof_is_balanced_each_paragraph() {
        let html = render("前［＃太字］中\n\nもっと\n\n最後");
        assert_eq!(
            html.matches("<b").count(),
            html.matches("</b>").count(),
            "bold must be globally balanced to EOF: {html}",
        );
        assert_eq!(
            html.matches("<b").count(),
            3,
            "the never-closed bold reopens in each of the 3 paragraphs: {html}",
        );
        assert!(
            bold_never_straddles_p_close(&html),
            "no open <b> may straddle </p> anywhere: {html}",
        );
        assert!(
            html.contains("<p><b class=\"aozora-futoji\">もっと</b></p>"),
            "a trailing paragraph is fully bold: {html}",
        );
    }

    #[test]
    fn bold_close_marker_after_paragraph_break_still_pairs() {
        let html = render("前［＃太字］中\n\n後［＃太字終わり］尾");
        assert_eq!(
            html.matches("<b").count(),
            html.matches("</b>").count(),
            "bold must be balanced (close marker pairs): {html}",
        );
        assert!(
            bold_never_straddles_p_close(&html),
            "no open <b> may straddle </p>: {html}",
        );
        assert!(
            html.contains("<p>前<b class=\"aozora-futoji\">中</b></p>"),
            "first paragraph is bold and closes before </p>: {html}",
        );
        assert!(
            html.contains("<p><b class=\"aozora-futoji\">後</b>尾</p>"),
            "second paragraph reopens bold, the close marker ends it, 尾 is plain: {html}",
        );
    }

    #[test]
    fn bold_close_in_paragraph_gap_ends_emphasis() {
        let html = render("前［＃太字］中\n\n［＃太字終わり］後");
        assert_eq!(
            html.matches("<b").count(),
            html.matches("</b>").count(),
            "bold balanced: {html}",
        );
        assert!(
            html.contains("<p>前<b class=\"aozora-futoji\">中</b></p>")
                && html.contains("<p>後</p>"),
            "後 after a gap-close must be plain, not bold: {html}",
        );
        let nested = render("前［＃太字］あ［＃斜体］い\n\n［＃斜体終わり］［＃太字終わり］後");
        assert_eq!(
            nested.matches("<b").count(),
            nested.matches("</b>").count(),
            "nested bold balanced: {nested}",
        );
        assert_eq!(
            nested.matches("<i").count(),
            nested.matches("</i>").count(),
            "nested italic balanced: {nested}",
        );
        assert!(
            nested.contains("<p>後</p>"),
            "後 after nested gap-closes must be plain: {nested}",
        );
    }

    #[test]
    fn deep_gap_closes_drain_pending_inline_scopes() {
        let depth = 4_096;
        let mut state = RenderState::default();
        let mut out = String::new();
        let bold = RegionFormat::Bold { padded: false };
        for _ in 0..depth {
            state
                .open_container(bold, &mut out)
                .expect("render into String is infallible");
        }
        state
            .close_paragraph(&mut out)
            .expect("render into String is infallible");
        for _ in 0..depth {
            state
                .close_container(true, &mut out)
                .expect("render into String is infallible");
        }
        assert!(state.reopen_after_para.is_empty());
        assert_eq!(state.open_stack, Vec::<RegionFormat>::new());

        let suffix = out.len();
        state
            .ensure_in_paragraph(&mut out)
            .expect("render into String is infallible");
        state
            .finish(&mut out)
            .expect("render into String is infallible");
        assert_eq!(&out[suffix..], "<p></p>\n");
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

    #[test]
    fn before_block_emit_closes_open_paragraph() {
        let mut st = RenderState::default();
        let mut out = String::new();
        st.ensure_in_paragraph(&mut out).expect("infallible");
        out.push('X');
        st.before_block_emit(&mut out).expect("infallible");
        assert_eq!(out, "<p>X</p>\n");
    }

    #[test]
    fn container_open_pins_each_region_markup() {
        let cases = [
            (
                RegionFormat::LineWidth(LineWidth(NonZeroU8::new(7).unwrap())),
                r#"<div class="aozora-container aozora-container-line-width" data-width="7">"#,
            ),
            (
                RegionFormat::Framed(EnclosureKind::Rule),
                r#"<div class="aozora-container aozora-container-keigakomi">"#,
            ),
            (
                RegionFormat::Gothic { padded: true },
                r#"<div class="aozora-container aozora-container-goshikku">"#,
            ),
            (
                RegionFormat::Gothic { padded: false },
                r#"<span class="aozora-goshikku">"#,
            ),
            (
                RegionFormat::Horizontal,
                r#"<div class="aozora-container aozora-container-yokogumi">"#,
            ),
            (
                RegionFormat::SmallScript(BoutenPosition::Left),
                r#"<span class="aozora-kogaki-left">"#,
            ),
            (
                RegionFormat::SmallScript(BoutenPosition::Right),
                r#"<span class="aozora-kogaki-right">"#,
            ),
            (
                RegionFormat::Caption { padded: false },
                r#"<span class="aozora-caption">"#,
            ),
            (
                RegionFormat::Caption { padded: true },
                r#"<div class="aozora-container aozora-caption">"#,
            ),
        ];
        for (kind, expected) in cases {
            assert_eq!(open_tag(kind), expected, "open {kind:?}");
        }
    }

    #[test]
    fn container_close_pins_each_region_markup() {
        let cases = [
            (
                RegionFormat::Bouten {
                    kind: BoutenKind::Goma,
                    position: BoutenPosition::Right,
                },
                "</em>",
            ),
            (RegionFormat::SmallScript(BoutenPosition::Right), "</span>"),
            (RegionFormat::SmallScript(BoutenPosition::Left), "</span>"),
            (RegionFormat::Caption { padded: false }, "</span>"),
            (RegionFormat::Gothic { padded: false }, "</span>"),
        ];
        for (kind, expected) in cases {
            assert_eq!(close_tag(kind), expected, "close {kind:?}");
        }
    }

    #[test]
    fn heading_tag_maps_level_and_window_style() {
        assert_eq!(
            heading_tag(HeadingKind::Large, HeadingStyle::Standard),
            "h1"
        );
        assert_eq!(
            heading_tag(HeadingKind::Medium, HeadingStyle::Standard),
            "h2"
        );
        assert_eq!(
            heading_tag(HeadingKind::Small, HeadingStyle::Standard),
            "h3"
        );
        assert_eq!(
            heading_tag(HeadingKind::Medium, HeadingStyle::Window),
            "div"
        );
    }

    #[test]
    fn render_line_pins_each_line_directive_markup() {
        let cases = [
            (
                LineFormat::Indent {
                    amount: 2,
                    end_offset: None,
                },
                r#"<span class="aozora-indent aozora-indent-2" data-amount="2"></span>"#,
            ),
            (
                LineFormat::Indent {
                    amount: 2,
                    end_offset: Some(3),
                },
                r#"<span class="aozora-indent aozora-indent-2 aozora-align-end aozora-align-end-3" data-amount="2" data-offset="3"></span>"#,
            ),
            (
                LineFormat::AlignEnd { offset: 0 },
                r#"<span class="aozora-align-end" data-offset="0"></span>"#,
            ),
            (
                LineFormat::AlignEnd { offset: 5 },
                r#"<span class="aozora-align-end aozora-align-end-5" data-offset="5"></span>"#,
            ),
        ];
        for (lf, expected) in cases {
            assert_eq!(line_tag(lf), expected, "line {lf:?}");
        }
    }

    #[test]
    fn parse_sashie_dimensions_pins_digit_pairs() {
        assert_eq!(parse_sashie_dimensions("横100×縦200"), Some(("100", "200")));
        assert_eq!(parse_sashie_dimensions("横100縦200"), None);
        assert_eq!(parse_sashie_dimensions("100×縦200"), None);
        assert_eq!(parse_sashie_dimensions("横100×200"), None);
        assert_eq!(parse_sashie_dimensions("横10a×縦200"), None);
        assert_eq!(parse_sashie_dimensions("横100×縦20b"), None);
        assert_eq!(parse_sashie_dimensions("横×縦200"), None);
    }

    #[test]
    fn escape_text_advances_cursor_past_escaped_char() {
        let mut out = String::new();
        escape_text("a<b", &mut out).expect("infallible");
        assert_eq!(out, "a&lt;b");
    }

    #[test]
    fn html_entity_pins_each_escape() {
        assert_eq!(html_entity('<'), "&lt;");
        assert_eq!(html_entity('>'), "&gt;");
        assert_eq!(html_entity('&'), "&amp;");
        assert_eq!(html_entity('"'), "&quot;");
        assert_eq!(html_entity('\''), "&#x27;");
    }
}
