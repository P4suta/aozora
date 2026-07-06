//! Block-level HTML render state + text escaper.
//!
//! The shared, AST-free machinery the owned HTML renderer
//! ([`crate::html`]) drives: the two-state paragraph machine
//! ([`RenderState`]) that emits `<p>` / `<br />` / container brackets from the
//! walk's sentinel + newline events, and the bulk-copy [`escape_text_chunk`]
//! pass over plain runs. Container open/close tags route through the
//! lifetime-free [`render_container`], so the byte spelling stays
//! single-source.

use core::fmt::{self, Write};

use aozora_syntax::{
    BlockStyles, Container, HeadingKind, HeadingStyle, IndentBlock, IndentLayout, LineFormat,
    RegionFormat,
};
use memchr::{memchr_iter, memchr3_iter};

use crate::classes;

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
    /// Inline containers (`kind.is_inline()`) closed at a paragraph boundary,
    /// awaiting reopen in the next paragraph. An inline container is phrasing
    /// content, so a still-open one must not straddle `</p>`; but popping it off
    /// `open_stack` would desync a later `［＃…終わり］` close marker. So
    /// [`Self::close_paragraph`] closes it top-down and records it here, and
    /// [`Self::ensure_in_paragraph`] reopens it (re-pushing onto `open_stack`) in
    /// the next paragraph — keeping the stack consistent so the eventual close
    /// still pairs. A never-closed inline container renders balanced in each
    /// paragraph to EOF (#420).
    reopen_after_para: Vec<RegionFormat>,
    /// Count of open inline-warichu spans (`［＃割り注］`) awaiting their close
    /// (`［＃割り注終わり］`). A warichu span is phrasing content, so it must
    /// never straddle a `</p>` or `</div>`: [`Self::close_paragraph`] drains any
    /// still-open span before closing the paragraph, and [`Self::close_warichu`]
    /// absorbs a stray close with no matching open. Sources that mismatch the
    /// block- and inline-warichu forms (9 corpus works, #415) rely on this to
    /// stay balanced.
    warichu_depth: u32,
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
            // Reopen any inline containers `close_paragraph` closed at the last
            // paragraph boundary (#420). `pop()` yields them in reverse of the
            // top-down push order, i.e. outermost-first, restoring the original
            // nesting; re-pushing onto `open_stack` keeps the eventual close
            // paired. This runs only for real paragraphs — the `in_heading`
            // early return above skips it.
            while let Some(kind) = self.reopen_after_para.pop() {
                render_container(Container { kind }, true, out)?;
                self.open_stack.push(kind);
            }
        }
        Ok(())
    }

    pub(crate) fn close_paragraph<W: Write>(&mut self, out: &mut W) -> fmt::Result {
        // A warichu span is phrasing content that must close before the
        // enclosing paragraph (#415, Case 2): drain any still-open span here,
        // the single choke-point every block-leaf / container / finish path
        // routes through (via `before_block_emit` and `drain_open_containers`).
        self.drain_open_warichu(out)?;
        if self.in_paragraph {
            // An inline container is phrasing content and sits at the TOP of
            // `open_stack` (any block container is below it), so it must not
            // straddle `</p>` either (#420). Close each open inline container
            // top-down here and remember it, so `ensure_in_paragraph` can reopen
            // it in the next paragraph — re-pushing onto `open_stack` keeps a
            // later close marker paired.
            while let Some(&kind) = self.open_stack.last() {
                if !kind.is_inline() {
                    break;
                }
                self.open_stack.pop();
                render_container(Container { kind }, false, out)?;
                self.reopen_after_para.push(kind);
            }
            out.write_str("</p>\n")?;
            self.in_paragraph = false;
            self.pending_block_separator = false;
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

    /// Emit a container's opening tag, honouring its content model. An inline
    /// container stays in the current paragraph; a phrasing-content container (a
    /// heading) flushes the paragraph and holds its content inline under the
    /// `<hN>` (`in_heading`); every other block container flushes and brackets
    /// its content as block paragraphs.
    pub(crate) fn open_container<W: Write>(
        &mut self,
        kind: RegionFormat,
        out: &mut W,
    ) -> fmt::Result {
        self.open_stack.push(kind);
        let container = Container { kind };
        if kind.is_inline() {
            self.ensure_in_paragraph(out)?;
            return render_container(container, true, out);
        }
        self.before_block_emit(out)?;
        render_container(container, true, out)?;
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
    pub(crate) fn close_container<W: Write>(
        &mut self,
        closing_inline: bool,
        out: &mut W,
    ) -> fmt::Result {
        // An inline close marker (`［＃太字終わり］` etc.) that lands in the gap
        // between a paragraph boundary and the next text cancels a pending
        // reopen instead of no-oping (#420). `close_paragraph` already drained
        // the paragraph's inline containers into `reopen_after_para`, so the
        // stack is empty (or holds only the enclosing block) and a plain
        // `open_stack.pop()` would silently lose the close — and the next
        // paragraph would then wrongly re-apply the emphasis. The front entry
        // is the innermost (closed first), matching the inner-first close
        // order; a *block* close (`closing_inline == false`) still pops its
        // region off the stack as usual, so block markup is unaffected.
        if closing_inline && !self.reopen_after_para.is_empty() {
            self.reopen_after_para.remove(0);
            return Ok(());
        }
        let Some(kind) = self.open_stack.pop() else {
            return Ok(());
        };
        let container = Container { kind };
        if kind.is_inline() {
            self.ensure_in_paragraph(out)?;
            return render_container(container, false, out);
        }
        if kind.content_is_phrasing() {
            self.in_heading = false;
        } else {
            self.before_block_emit(out)?;
        }
        render_container(container, false, out)?;
        self.after_block_emit();
        Ok(())
    }

    /// Close every container left open at end-of-input. A source may open a
    /// region (`［＃ここから字下げ］`) without a matching close; the AST and
    /// `to_source` preserve that imbalance verbatim, but `to_html` must still
    /// emit valid, balanced markup — the unclosed region renders as extending
    /// to the end of the document. Mirrors the per-marker [`Self::close_container`].
    pub(crate) fn drain_open_containers<W: Write>(&mut self, out: &mut W) -> fmt::Result {
        while !self.open_stack.is_empty() {
            // EOF drain: no close marker, so pop the stack open-authoritatively.
            self.close_container(false, out)?;
        }
        Ok(())
    }

    /// Open an inline-warichu span (`［＃割り注］`), emitting its
    /// `<span class="aozora-warichu">` and recording the open so its close is
    /// balanced. The byte spelling matches the per-node fallback in
    /// [`crate::render_node`], so well-formed warichu output is unchanged.
    pub(crate) fn open_warichu<W: Write>(&mut self, out: &mut W) -> fmt::Result {
        out.write_str(r#"<span class="aozora-warichu">"#)?;
        self.warichu_depth += 1;
        Ok(())
    }

    /// Close one inline-warichu span (`［＃割り注終わり］`). A close with no
    /// matching open — a source that mismatches the block- and inline-warichu
    /// forms (#415, Case 1) — is absorbed as a no-op rather than emitting a stray
    /// `</span>`.
    pub(crate) fn close_warichu<W: Write>(&mut self, out: &mut W) -> fmt::Result {
        if self.warichu_depth > 0 {
            out.write_str("</span>")?;
            self.warichu_depth -= 1;
        }
        Ok(())
    }

    /// Close every warichu span left open when the paragraph / document ends —
    /// an inline `［＃割り注］` with no matching inline close (#415, Case 2). The
    /// span renders as extending to the paragraph boundary, mirroring
    /// [`Self::drain_open_containers`] for unclosed regions.
    pub(crate) fn drain_open_warichu<W: Write>(&mut self, out: &mut W) -> fmt::Result {
        while self.warichu_depth > 0 {
            out.write_str("</span>")?;
            self.warichu_depth -= 1;
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

        if cursor < pos {
            out.write_str(&chunk[cursor..pos])?;
        }
        let entity = match bytes[pos] {
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'&' => "&amp;",
            b'"' => "&quot;",
            // Hex form `&#x27;` matches `escape_text` so the
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

// ── HTML container / heading / line tag byte-spelling (formerly render_node.rs) ──

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

/// Emit a container's opening tag. Block containers render a
/// `<div class="aozora-container …">`; the inline range forms (bouten,
/// bare 太字 / 斜体, 小書き) render their inline element directly.
#[allow(
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
            // Exhaustive destructure (no `..`) so a new decoration is
            // compiler-flagged here rather than silently dropped from the markup.
            let BlockStyles {
                bold,
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
            // #78 secondary line-layout: 字組み grid gets its own class,
            // 字詰め reuses the standalone line-width class (same semantics).
            match layout {
                IndentLayout::Kumi(_) => {
                    writer.write_str(" aozora-container-line-kumi")?;
                }
                IndentLayout::LineWidth(_) => {
                    writer.write_str(" aozora-container-line-width")?;
                }
                IndentLayout::None => {}
            }
            // #78 co-applied decorative styles — flat classes on the same
            // `<div>` (close stays a single `</div>`), reusing each
            // attribute's standalone-container class so one stylesheet rule
            // serves both forms. Canonical order = bold, horizontal, framed,
            // font (matches `BlockStyles::iter_formats` / the serializer).
            if bold {
                writer.write_str(" aozora-container-futoji")?;
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
            // Range-form 傍点 / 傍線: an inline `<em>` matching the
            // forward-reference bouten markup so a stylesheet picks the
            // same per-variant treatment.
            write!(
                writer,
                r#"<em class="aozora-bouten aozora-bouten-{kind} aozora-bouten-{pos}">"#,
                kind = classes::bouten_kind_slug(kind),
                pos = classes::bouten_position_slug(position),
            )
        }
        // 太字 / 斜体. The bare inline range (`block: false`) uses the
        // same presentational `<b>` / `<i>` element as the
        // forward-reference [`render_emphasis`] leaf. The ここから-block
        // form (`block: true`) wraps whole paragraphs, so it takes a
        // block `<div>` (an inline `<b>` around `<p>` would be invalid),
        // following the indent / keigakomi container convention; the
        // `aozora-container-futoji` / `-shatai` class carries the styling.
        RegionFormat::Bold { padded: false } => writer.write_str(r#"<b class="aozora-futoji">"#),
        RegionFormat::Italic { padded: false } => writer.write_str(r#"<i class="aozora-shatai">"#),
        RegionFormat::Bold { padded: true } => {
            writer.write_str(r#"<div class="aozora-container aozora-container-futoji">"#)
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
        // Paired / block heading — same element as the forward-reference
        // leaf, but wrapping the delimited content (phrasing).
        RegionFormat::Heading { level, style, .. } => write_heading_open(level, style, writer),
        // 小書き range — inline `<span>`, matching the forward-reference
        // small-script leaf classes.
        RegionFormat::SmallScript(aozora_syntax::BoutenPosition::Left) => {
            writer.write_str(r#"<span class="aozora-kogaki-left">"#)
        }
        RegionFormat::SmallScript(_) => writer.write_str(r#"<span class="aozora-kogaki-right">"#),
        // Caption: inline `<span>` for the bare range, block `<div>` for ここから.
        RegionFormat::Caption { padded: false } => {
            writer.write_str(r#"<span class="aozora-caption">"#)
        }
        RegionFormat::Caption { padded: true } => {
            writer.write_str(r#"<div class="aozora-container aozora-caption">"#)
        }
        // 縦中横 range — inline `<span>`, matching the forward-reference
        // combine-upright leaf class so a stylesheet treats both alike.
        RegionFormat::CombineUpright => {
            writer.write_str(r#"<span class="aozora-combine-upright">"#)
        }
        _ => writer.write_str(r#"<div class="aozora-container">"#),
    }
}

/// Emit a container's closing tag — `</em>` / `</b>` / `</i>` for the inline
/// range forms, the heading element for a block heading, `</div>` otherwise.
fn render_container_close<W: Write>(kind: RegionFormat, writer: &mut W) -> fmt::Result {
    match kind {
        RegionFormat::Heading { level, style, .. } => write_heading_close(level, style, writer),
        _ => writer.write_str(match kind {
            RegionFormat::Bouten { .. } => "</em>",
            RegionFormat::Bold { padded: false } => "</b>",
            RegionFormat::Italic { padded: false } => "</i>",
            RegionFormat::SmallScript(_)
            | RegionFormat::Caption { padded: false }
            | RegionFormat::CombineUpright => "</span>",
            _ => "</div>",
        }),
    }
}

/// Render a `［＃挿絵（file）入る］` illustration as a semantic
/// Parse the bundled `横W×縦H` pixel-size note into `(width, height)` —
/// both runs of ASCII digits. Returns `None` for any other shape (the
/// dimensions then carry no HTML width/height hint).
pub(crate) fn parse_sashie_dimensions(dims: &str) -> Option<(&str, &str)> {
    let (w, h) = dims.split_once('×')?;
    let w = w.strip_prefix('横')?;
    let h = h.strip_prefix('縦')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (digits(w) && digits(h)).then_some((w, h))
}

/// The HTML tag for a heading. The 窓 (window) style is an inset block, not an
/// outline level, so it takes a `<div>`; otherwise the 大 / 中 / 小 level maps
/// to the semantic `<h1>`–`<h3>` outline tag.
fn heading_tag(kind: HeadingKind, style: HeadingStyle) -> &'static str {
    if matches!(style, HeadingStyle::Window) {
        "div"
    } else {
        match kind {
            HeadingKind::Medium => "h2",
            HeadingKind::Small => "h3",
            _ => "h1",
        }
    }
}

/// Write a heading's opening tag — `<hN>` / `<div>` with an
/// `aozora-heading-<large|medium|small>` class plus an
/// `aozora-heading-<same-line|window>` modifier for a non-standard style.
/// Shared by the forward-reference leaf `render_aozora_heading` and the
/// paired / block [`RegionFormat::Heading`] container so both render
/// identically.
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

/// Write a heading's closing tag (matching [`write_heading_open`]).
pub(crate) fn write_heading_close<W: Write>(
    kind: HeadingKind,
    style: HeadingStyle,
    writer: &mut W,
) -> fmt::Result {
    write!(writer, "</{}>", heading_tag(kind, style))
}

/// Render a single-line layout directive (字下げ / 地付き / 中央 / 罫囲み) as
/// a zero-width hook span; the actual layout is left to a stylesheet.
pub(crate) fn render_line<W: Write>(lf: LineFormat, writer: &mut W) -> fmt::Result {
    match lf {
        LineFormat::Indent {
            amount,
            end_offset: None,
        } => write!(
            writer,
            r#"<span class="aozora-indent aozora-indent-{amount}" data-amount="{amount}"></span>"#,
        ),
        // Both-margin compound: the head-indent classes plus the existing
        // align-end classes for the foot-edge lift (reused, not new tokens).
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
        // 罫囲み (line) routes through the paired 罫囲み container in practice,
        // so this hook is classifier-unreachable (corpus render-correctness
        // I-C confirms 0 occurrences across 17,889 works); emit the *declared*
        // inline-keigakomi class so the output stays valid if ever reached.
        LineFormat::Framed(_) => {
            writer.write_str(r#"<span class="aozora-keigakomi-inline"></span>"#)
        }
        LineFormat::Bold => writer.write_str(r#"<span class="aozora-line-futoji"></span>"#),
        // Absolute font-size line marker; `、太字` adds the line-bold class too.
        LineFormat::FontSizeAbsolute { size, bold } => {
            let slug = aozora_spec::roman_slug(size.keyword()).unwrap_or("font-small");
            if bold {
                write!(
                    writer,
                    r#"<span class="aozora-line-{slug} aozora-line-futoji"></span>"#,
                )
            } else {
                write!(writer, r#"<span class="aozora-line-{slug}"></span>"#)
            }
        }
        // `LineFormat` is `#[non_exhaustive]`; forward-compat skip.
        _ => Ok(()),
    }
}

/// Minimal HTML5 text escape — five structural ASCII characters.
/// Apostrophe uses the hex form `&#x27;`; the contract is pinned by
/// the integration tests in this crate.
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
    use crate::render_html;
    use pretty_assertions::assert_eq;

    fn render(src: &str) -> String {
        render_html(&aozora_pipeline::lex(src))
    }

    #[test]
    fn plain_paragraph_wraps_in_p() {
        assert_eq!(render("Hello."), "<p>Hello.</p>\n");
    }

    /// A well-formed inline warichu pair emits the same balanced span it always
    /// did — a byte-identity guard that the `RenderState`-owned depth machinery
    /// (#415) does not perturb the correct case.
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

    /// #415 Case 1: a block-form warichu open (`［＃ここから割り注］`) paired with an
    /// inline-form close (`［＃割り注終わり］`) must not leak a stray `</span>` — the
    /// unmatched inline close is absorbed as a no-op.
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
        // The block warichu container is still balanced.
        assert_eq!(html.matches("<div").count(), html.matches("</div>").count());
    }

    /// #415 Case 2: an inline-form warichu open (`［＃割り注］`) paired with a
    /// block-form close (`［＃ここで割り注終わり］`) must have its span drained before
    /// the paragraph closes — the open `<span>` never straddles `</p>`.
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
        // The drained close lands before the paragraph boundary, not after it.
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
        // Byte-identity regression guard for #420: a well-formed inline container
        // opens AND closes within its paragraph, so the paragraph-boundary
        // reopen machinery is a no-op — the top of `open_stack` is never inline
        // at `close_paragraph`. Output must be exactly as before the fix.
        let html = render("前［＃太字］中［＃太字終わり］後");
        assert_eq!(
            html, "<p>前<b class=\"aozora-futoji\">中</b>後</p>\n",
            "inline container must stay within the paragraph",
        );
    }

    /// True iff `<b>` is balanced at every `</p>` boundary — i.e. no open bold
    /// ever straddles a paragraph close (#420). A `</b>` (`<`,`/`,`b`,`>`) never
    /// contains the substring `<b`, so `matches("<b")` counts only opening tags.
    fn bold_never_straddles_p_close(html: &str) -> bool {
        let mut cursor = 0;
        while let Some(rel) = html[cursor..].find("</p>") {
            let end = cursor + rel; // start of this `</p>`
            let prefix = &html[..end];
            if prefix.matches("<b").count() != prefix.matches("</b>").count() {
                return false;
            }
            cursor = end + "</p>".len();
        }
        true
    }

    /// #420: an inline 太字 container the source never closes must not leave an
    /// open `<b>` straddling `</p>` across a paragraph break. The container is
    /// closed before `</p>` and reopened in the next paragraph, so bold is
    /// globally balanced and balanced at the `</p>` boundary.
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

    /// #420: an unclosed inline 太字 that reaches EOF renders balanced, with each
    /// trailing paragraph bold and no bold straddling any `</p>`.
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

    /// #420: a 太字 opened before a paragraph break and closed with
    /// `［＃太字終わり］` in a later paragraph must still pair — the reopened
    /// container stays on `open_stack`, so the close marker finds its match and
    /// text after the close is no longer bold.
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

    /// #420: an inline close marker landing in the *gap* between a paragraph
    /// break and the next text (no intervening text) ends the emphasis — it
    /// must cancel the pending reopen, not silently no-op and then wrongly
    /// re-apply the emphasis to the following paragraph. Regression guard for
    /// the reopen-cancel path.
    #[test]
    fn bold_close_in_paragraph_gap_ends_emphasis() {
        // Single: the close cancels the pending reopen, so 後 is plain.
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
        // Nested: both closes in the gap end both emphases (inner-first),
        // leaving 後 plain — the outer/block markup is untouched.
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
