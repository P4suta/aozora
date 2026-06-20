//! HTML rendering for individual borrowed-AST nodes.
//!
//! Per-node renderer parameterised over the source/arena lifetime
//! `'src`. Public entry point: [`render`].

use core::fmt::{self, Write};

use aozora_syntax::borrowed::{
    AngleQuote, Bouten, Content, Directive, Emphasis, Gaiji, Heading, HeadingHint, Illustration,
    Kaeriten, MarginNote, Node, Ruby, Segment,
};
use aozora_syntax::{
    AlignEnd, Container, ContainerKind, DirectiveKind, EmphasisKind, HeadingKind, HeadingStyle,
    Indent, RubySide,
};

use crate::classes;

/// Render a single borrowed [`Node`] into `writer`.
///
/// `entering` follows the standard tree-walker enter/exit convention:
/// inline / leaf nodes emit their markup only on `entering == true`
/// and produce nothing on the exit pass. Container nodes
/// ([`Node::Container`]) emit an opening tag on enter and a
/// closing tag on exit — the calling block walker drives children
/// between the two events.
///
/// # Errors
///
/// Propagates formatter write errors.
pub fn render<W: Write>(node: Node<'_>, entering: bool, writer: &mut W) -> fmt::Result {
    match node {
        Node::Container(c) => render_container(c, entering, writer),
        _ if !entering => Ok(()),
        Node::Ruby(r) => render_ruby(r, writer),
        Node::Bouten(b) => render_bouten(b, writer),
        Node::Emphasis(e) => render_emphasis(e, writer),
        Node::MarginNote(s) => render_side_note(s, writer),
        Node::CombineUpright(t) => {
            writer.write_str(r#"<span class="aozora-combine-upright">"#)?;
            render_content(t.text.get(), writer)?;
            writer.write_str("</span>")
        }
        Node::Gaiji(g) => render_gaiji(g, writer),
        Node::Indent(i) => render_indent(i, writer),
        Node::AlignEnd(a) => render_align_end(a, writer),
        Node::Center(_) => render_center(writer),
        Node::PageBreak => writer.write_str(r#"<div class="aozora-page-break"></div>"#),
        Node::SectionBreak(k) => {
            // Single source of truth for the romaji slug: the spec slug
            // table, keyed by the canonical 青空文庫 keyword.
            let slug = aozora_spec::roman_slug(k.keyword()).unwrap_or("other");
            write!(
                writer,
                r#"<div class="aozora-section-break aozora-section-break-{slug}"></div>"#,
            )
        }
        Node::Directive(a) => render_annotation(a, writer),
        Node::Kaeriten(k) => render_kaeriten(k, writer),
        Node::AngleQuote(d) => render_angle_quote(d, writer),
        Node::Illustration(s) => render_sashie(s, writer),
        Node::Heading(h) => render_aozora_heading(h, writer),
        Node::HeadingHint(h) => render_heading_hint(h, writer),
        // Other variants — emit a fallback comment so the rendered
        // HTML stays diagnosable. Mirrors the owned renderer's
        // catch-all behavior for Heading / HeadingHint / Illustration /
        // Warichu / Framed (which the legacy renderer also routes
        // through `fallback`).
        _ => fallback(node, writer),
    }
}

fn render_ruby<W: Write>(r: &Ruby<'_>, writer: &mut W) -> fmt::Result {
    writer.write_str("<ruby>")?;
    render_content(r.base.get(), writer)?;
    // A left-side ruby (saidoku building block) marks its `<rt>` with a class
    // so a stylesheet can place the reading below; the right-side form is
    // unchanged.
    writer.write_str(match r.side {
        RubySide::Left => r#"<rp>(</rp><rt class="aozora-ruby-left">"#,
        _ => "<rp>(</rp><rt>",
    })?;
    render_content(r.reading.get(), writer)?;
    writer.write_str("</rt><rp>)</rp></ruby>")
}

fn render_side_note<W: Write>(s: &MarginNote<'_>, writer: &mut W) -> fmt::Result {
    // A 注記 attaches a left-side editorial note to the base — like a
    // left-side ruby in layout, but a note rather than a reading, so it
    // reuses the ruby box with a distinct `aozora-margin-note` class.
    writer.write_str("<ruby>")?;
    render_content(s.base.get(), writer)?;
    writer.write_str(r#"<rp>(</rp><rt class="aozora-margin-note">"#)?;
    render_content(s.note.get(), writer)?;
    writer.write_str("</rt><rp>)</rp></ruby>")
}

fn render_bouten<W: Write>(b: &Bouten<'_>, writer: &mut W) -> fmt::Result {
    write!(
        writer,
        r#"<em class="aozora-bouten aozora-bouten-{kind} aozora-bouten-{pos}">"#,
        kind = classes::bouten_kind_slug(b.kind),
        pos = classes::bouten_position_slug(b.position),
    )?;
    render_content(b.target.get(), writer)?;
    writer.write_str("</em>")
}

/// Render a forward-reference emphasis run. 太字 maps to the presentational
/// `<b>` element, 斜体 to `<i>`, 上付き/下付き小文字 to `<sup>` / `<sub>`,
/// 行右/行左小書き to a side `<span>`, and `N段階大きな/小さな文字` to a
/// `<span class="aozora-font-larger|smaller" data-steps="N">` — each carries
/// an `aozora-*` class so a stylesheet can theme them, and none collides with
/// the `<em class="aozora-bouten …">` that [`render_bouten`] owns.
fn render_emphasis<W: Write>(e: &Emphasis<'_>, writer: &mut W) -> fmt::Result {
    // 文字サイズ carries a magnitude, so its open tag is dynamic.
    if let EmphasisKind::FontSize { steps } = e.kind {
        let (class, magnitude) = if steps >= 0 {
            ("aozora-font-larger", steps)
        } else {
            ("aozora-font-smaller", -steps)
        };
        write!(writer, r#"<span class="{class}" data-steps="{magnitude}">"#)?;
        render_content(e.text.get(), writer)?;
        return writer.write_str("</span>");
    }
    // The HTML element is semantic (italic→<i>, super/sub→<sup>/<sub>,
    // 太字→<b>, the small-glyph / inline-box / caption forms→<span>) and
    // stays here; the `aozora-*` class slug comes from the single source
    // of truth (the spec slug table), keyed by the canonical keyword.
    let (el, close) = match e.kind {
        EmphasisKind::Italic => ("i", "</i>"),
        EmphasisKind::SuperScript => ("sup", "</sup>"),
        EmphasisKind::SubScript => ("sub", "</sub>"),
        EmphasisKind::SmallRight
        | EmphasisKind::SmallLeft
        | EmphasisKind::KeigakomiInline
        | EmphasisKind::HorizontalInline
        | EmphasisKind::Caption => ("span", "</span>"),
        // `EmphasisKind` is `#[non_exhaustive]`; 太字 and any future
        // weight default to the bold element.
        _ => ("b", "</b>"),
    };
    let slug = aozora_spec::roman_slug(e.kind.keyword()).unwrap_or("bold");
    write!(writer, r#"<{el} class="aozora-{slug}">"#)?;
    render_content(e.text.get(), writer)?;
    writer.write_str(close)
}

/// Render a [`Content`] by walking its segments in order.
fn render_content<W: Write>(content: Content<'_>, writer: &mut W) -> fmt::Result {
    for seg in content {
        match seg {
            Segment::Text(t) => escape_text(t, writer)?,
            Segment::Gaiji(g) => render_gaiji(g, writer)?,
            Segment::Directive(a) => render_annotation(a, writer)?,
            // Borrowed `Segment` is `#[non_exhaustive]`; future variants
            // emit nothing until a dedicated renderer lands.
            _ => {}
        }
    }
    Ok(())
}

fn render_gaiji<W: Write>(g: &Gaiji<'_>, writer: &mut W) -> fmt::Result {
    // The renderer exposes two data attributes so downstream HTML
    // consumers (aozora-obsidian, afm sibling plugins, themed
    // sites) can switch gaiji presentation between
    // image / description / codepoint at view time without a second
    // parser pass:
    //
    //   - `data-codepoint` lists the resolved Unicode scalar(s) as
    //     space-separated `U+XXXX` entries (single-char cells emit
    //     one entry; 25 JIS X 0213 combining-sequence cells emit
    //     one per scalar).
    //   - `data-description` carries the raw 注記 text when the
    //     gaiji could not be resolved to Unicode and the renderer
    //     fell back to the description payload.
    //
    // The `<span class="aozora-gaiji">…</span>` wrapper plus the
    // displayed text content stay byte-for-byte equivalent to the
    // pre-Plan-B.5 shape — the data attributes are additive.
    if let Some(resolved) = g.ucs {
        writer.write_str(r#"<span class="aozora-gaiji" data-codepoint=""#)?;
        // Round-trip Resolved through a tiny String buffer so we
        // can iterate its scalars without re-implementing the
        // Char/Multi enum split. `write_to` is the public
        // accessor and never fails into a String.
        let mut buf = String::with_capacity(8);
        resolved
            .write_to(&mut buf)
            .expect("Resolved::write_to into String never fails");
        let mut first = true;
        for c in buf.chars() {
            if !first {
                writer.write_char(' ')?;
            }
            first = false;
            write!(writer, "U+{:04X}", c as u32)?;
        }
        writer.write_str(r#"">"#)?;
        resolved.write_to(writer)?;
    } else {
        writer.write_str(r#"<span class="aozora-gaiji" data-description=""#)?;
        escape_text(g.description, writer)?;
        writer.write_str(r#"">"#)?;
        escape_text(g.description, writer)?;
    }
    writer.write_str("</span>")
}

fn render_annotation<W: Write>(a: &Directive<'_>, writer: &mut W) -> fmt::Result {
    match a.kind {
        DirectiveKind::WarichuOpen => return writer.write_str(r#"<span class="aozora-warichu">"#),
        DirectiveKind::WarichuClose => return writer.write_str("</span>"),
        _ => {}
    }
    writer.write_str(r#"<span class="aozora-directive" hidden>"#)?;
    escape_text(a.raw.as_str(), writer)?;
    writer.write_str("</span>")
}

fn render_kaeriten<W: Write>(k: &Kaeriten<'_>, writer: &mut W) -> fmt::Result {
    writer.write_str(r#"<sup class="aozora-kaeriten">"#)?;
    escape_text(k.mark.as_str(), writer)?;
    writer.write_str("</sup>")
}

fn render_container<W: Write>(c: Container, entering: bool, writer: &mut W) -> fmt::Result {
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
    reason = "one match arm per ContainerKind — splitting would scatter the \
              1:1 kind→markup mapping that mirrors emit_container_open"
)]
fn render_container_open<W: Write>(kind: ContainerKind, writer: &mut W) -> fmt::Result {
    match kind {
        ContainerKind::Indent {
            amount,
            wrap,
            center,
        } => {
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
            write!(writer, r#"" data-amount="{amount}""#)?;
            if let Some(w) = wrap {
                write!(writer, r#" data-wrap="{w}""#)?;
            }
            writer.write_str(">")
        }
        ContainerKind::AlignEnd { offset } => {
            write!(
                writer,
                r#"<div class="aozora-container aozora-container-align-end" data-offset="{offset}">"#,
            )
        }
        ContainerKind::LineWidth { width } => {
            write!(
                writer,
                r#"<div class="aozora-container aozora-container-line-width" data-width="{width}">"#,
            )
        }
        ContainerKind::Framed => {
            writer.write_str(r#"<div class="aozora-container aozora-container-keigakomi">"#)
        }
        ContainerKind::Warichu => {
            writer.write_str(r#"<div class="aozora-container aozora-container-warichu">"#)
        }
        ContainerKind::BoutenRange { kind, position } => {
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
        // `aozora-container-bold` / `-italic` class carries the styling.
        ContainerKind::Bold { block: false } => writer.write_str(r#"<b class="aozora-bold">"#),
        ContainerKind::Italic { block: false } => writer.write_str(r#"<i class="aozora-italic">"#),
        ContainerKind::Bold { block: true } => {
            writer.write_str(r#"<div class="aozora-container aozora-container-bold">"#)
        }
        ContainerKind::Italic { block: true } => {
            writer.write_str(r#"<div class="aozora-container aozora-container-italic">"#)
        }
        ContainerKind::Columns { count } => write!(
            writer,
            r#"<div class="aozora-container aozora-container-columns" data-columns="{count}">"#,
        ),
        ContainerKind::Table => {
            writer.write_str(r#"<div class="aozora-container aozora-container-table">"#)
        }
        ContainerKind::Horizontal => {
            writer.write_str(r#"<div class="aozora-container aozora-container-horizontal">"#)
        }
        ContainerKind::FontSize { steps } => {
            let (class, magnitude) = if steps >= 0 {
                ("aozora-container-font-larger", steps)
            } else {
                ("aozora-container-font-smaller", -steps)
            };
            write!(
                writer,
                r#"<div class="aozora-container {class}" data-steps="{magnitude}">"#,
            )
        }
        // Paired / block heading — same element as the forward-reference
        // leaf, but wrapping the delimited content (phrasing).
        ContainerKind::Heading { kind, style, .. } => write_heading_open(kind, style, writer),
        // 小書き range — inline `<span>`, matching the forward-reference
        // `EmphasisKind::SmallRight` / `SmallLeft` leaf classes.
        ContainerKind::SmallScript {
            side: aozora_syntax::BoutenPosition::Left,
        } => writer.write_str(r#"<span class="aozora-kogaki-left">"#),
        ContainerKind::SmallScript { .. } => {
            writer.write_str(r#"<span class="aozora-kogaki-right">"#)
        }
        // Caption: inline `<span>` for the bare range, block `<div>` for ここから.
        ContainerKind::Caption { block: false } => {
            writer.write_str(r#"<span class="aozora-caption">"#)
        }
        ContainerKind::Caption { block: true } => {
            writer.write_str(r#"<div class="aozora-container aozora-caption">"#)
        }
        // 縦中横 range — inline `<span>`, matching the forward-reference
        // [`CombineUpright`] leaf class so a stylesheet treats both alike.
        ContainerKind::CombineUprightRange => {
            writer.write_str(r#"<span class="aozora-combine-upright">"#)
        }
        _ => writer.write_str(r#"<div class="aozora-container">"#),
    }
}

/// Emit a container's closing tag — `</em>` / `</b>` / `</i>` for the inline
/// range forms, the heading element for a block heading, `</div>` otherwise.
fn render_container_close<W: Write>(kind: ContainerKind, writer: &mut W) -> fmt::Result {
    match kind {
        ContainerKind::Heading { kind, style, .. } => write_heading_close(kind, style, writer),
        _ => writer.write_str(match kind {
            ContainerKind::BoutenRange { .. } => "</em>",
            ContainerKind::Bold { block: false } => "</b>",
            ContainerKind::Italic { block: false } => "</i>",
            ContainerKind::SmallScript { .. }
            | ContainerKind::Caption { block: false }
            | ContainerKind::CombineUprightRange => "</span>",
            _ => "</div>",
        }),
    }
}

fn render_angle_quote<W: Write>(d: &AngleQuote<'_>, writer: &mut W) -> fmt::Result {
    writer.write_str(r#"<span class="aozora-angle-quote">《"#)?;
    render_content(d.content.get(), writer)?;
    writer.write_str("》</span>")
}

/// Render a `［＃挿絵（file）入る］` illustration as a semantic
/// Parse the bundled `横W×縦H` pixel-size note into `(width, height)` —
/// both runs of ASCII digits. Returns `None` for any other shape (the
/// dimensions then carry no HTML width/height hint).
fn parse_sashie_dimensions(dims: &str) -> Option<(&str, &str)> {
    let (w, h) = dims.split_once('×')?;
    let w = w.strip_prefix('横')?;
    let h = h.strip_prefix('縦')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (digits(w) && digits(h)).then_some((w, h))
}

/// `<figure>` carrying an `<img>` reference. The parser does not fetch
/// or embed pixels — `src` is the verbatim filename from the directive
/// and `alt` is left empty (the optional caption, when a future
/// captioned-form recogniser populates it, renders into `<figcaption>`).
/// `Illustration::is_block()` is `true`, so the block walker has already
/// flushed the surrounding paragraph before this fires.
fn render_sashie<W: Write>(s: &Illustration<'_>, writer: &mut W) -> fmt::Result {
    writer.write_str(r#"<figure class="aozora-illustration"><img src=""#)?;
    escape_text(s.file.as_str(), writer)?;
    writer.write_char('"')?;
    if let Some((w, h)) = s.dimensions.and_then(parse_sashie_dimensions) {
        write!(writer, r#" width="{w}" height="{h}""#)?;
    }
    // The general image form's leading description (図 / コンドル博士の図 …)
    // is the alt; the keyword 挿絵 form carries none, so alt stays empty.
    writer.write_str(r#" alt=""#)?;
    if let Some(description) = s.description {
        escape_text(description, writer)?;
    }
    writer.write_str(r#"" />"#)?;
    if let Some(caption) = s.caption {
        writer.write_str("<figcaption>")?;
        render_content(caption, writer)?;
        writer.write_str("</figcaption>")?;
    }
    writer.write_str("</figure>")
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
/// Shared by the forward-reference leaf [`render_aozora_heading`] and the
/// paired / block [`ContainerKind::Heading`] container so both render
/// identically.
fn write_heading_open<W: Write>(
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
fn write_heading_close<W: Write>(
    kind: HeadingKind,
    style: HeadingStyle,
    writer: &mut W,
) -> fmt::Result {
    write!(writer, "</{}>", heading_tag(kind, style))
}

/// Render a forward-reference promoted heading (leaf). The standard style adds
/// no modifier, so its markup is unchanged. `Heading::is_block()` is
/// `true`, so the block walker has flushed the surrounding paragraph before
/// this fires.
fn render_aozora_heading<W: Write>(h: &Heading<'_>, writer: &mut W) -> fmt::Result {
    write_heading_open(h.kind, h.style, writer)?;
    render_content(h.text.get(), writer)?;
    write_heading_close(h.kind, h.style, writer)
}

/// Render an *unpromoted* forward-reference heading hint. The referent is
/// not the bare preceding line, so the heading text stays in the flow and
/// this hidden inline marker records the intended outline level + target
/// for downstream promotion — no visible `<!-- … -->` placeholder leaks.
fn render_heading_hint<W: Write>(h: &HeadingHint<'_>, writer: &mut W) -> fmt::Result {
    write!(
        writer,
        r#"<span class="aozora-heading-hint" data-level="{level}""#,
        level = h.level,
    )?;
    // `data-style` is emitted only for a non-standard style, so a standard
    // hint's markup is unchanged.
    if let Some(style) = classes::heading_style_slug(h.style) {
        write!(writer, r#" data-style="{style}""#)?;
    }
    writer.write_str(r#" data-target=""#)?;
    escape_text(h.target.as_str(), writer)?;
    writer.write_str(r#"" hidden></span>"#)
}

fn render_indent<W: Write>(i: Indent, writer: &mut W) -> fmt::Result {
    write!(
        writer,
        r#"<span class="aozora-indent aozora-indent-{n}" data-amount="{n}"></span>"#,
        n = i.amount,
    )
}

fn render_align_end<W: Write>(a: AlignEnd, writer: &mut W) -> fmt::Result {
    if a.offset == 0 {
        writer.write_str(r#"<span class="aozora-align-end" data-offset="0"></span>"#)
    } else {
        write!(
            writer,
            r#"<span class="aozora-align-end aozora-align-end-{n}" data-offset="{n}"></span>"#,
            n = a.offset,
        )
    }
}

/// Render a single-line centring marker (`ページの左右中央` / `中央揃え`). A
/// zero-width hook; the actual centring is left to a stylesheet.
fn render_center<W: Write>(writer: &mut W) -> fmt::Result {
    writer.write_str(r#"<span class="aozora-center"></span>"#)
}

fn fallback<W: Write>(node: Node<'_>, writer: &mut W) -> fmt::Result {
    write!(writer, "<!-- {} -->", node.xml_node_name())
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
    use super::*;
    use aozora_syntax::alloc::BorrowedAllocator;
    use aozora_syntax::borrowed::{Arena, Node};
    use aozora_syntax::{AlignEnd, BoutenKind, BoutenPosition, DirectiveKind, Indent, SectionKind};

    fn render_node_to_string(node: Node<'_>) -> String {
        let mut out = String::new();
        render(node, true, &mut out).expect("fmt::Write into String never fails");
        out
    }

    #[test]
    fn ruby_emits_rp_rt_canonical_form() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let base = alloc.content_plain("青梅");
        let reading = alloc.content_plain("おうめ");
        let n = alloc.ruby(base, reading, true);
        assert_eq!(
            render_node_to_string(n),
            "<ruby>青梅<rp>(</rp><rt>おうめ</rt><rp>)</rp></ruby>"
        );
    }

    #[test]
    fn ruby_escapes_structural_characters() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let base = alloc.content_plain("<x>");
        let reading = alloc.content_plain("&y");
        let n = alloc.ruby(base, reading, true);
        let out = render_node_to_string(n);
        assert!(out.contains("&lt;x&gt;"));
        assert!(out.contains("&amp;y"));
    }

    #[test]
    fn page_break_is_self_contained_div() {
        let arena = Arena::new();
        let alloc = BorrowedAllocator::new(&arena);
        let n = alloc.page_break();
        assert_eq!(
            render_node_to_string(n),
            r#"<div class="aozora-page-break"></div>"#
        );
    }

    #[test]
    fn annotation_unknown_wraps_in_hidden_span() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let payload = alloc.make_directive("［＃改ページ］", DirectiveKind::Unknown);
        let n = alloc.annotation(payload);
        assert_eq!(
            render_node_to_string(n),
            r#"<span class="aozora-directive" hidden>［＃改ページ］</span>"#
        );
    }

    #[test]
    fn bouten_kind_and_position_slug() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let target = alloc.content_plain("可哀想");
        let n = alloc.bouten(BoutenKind::Goma, target, BoutenPosition::Right, false);
        assert_eq!(
            render_node_to_string(n),
            r#"<em class="aozora-bouten aozora-bouten-goma aozora-bouten-right">可哀想</em>"#
        );
    }

    #[test]
    fn bouten_black_triangle_uses_dedicated_slug() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let target = alloc.content_plain("規範");
        let n = alloc.bouten(
            BoutenKind::BlackTriangle,
            target,
            BoutenPosition::Right,
            false,
        );
        assert_eq!(
            render_node_to_string(n),
            r#"<em class="aozora-bouten aozora-bouten-kurosankaku aozora-bouten-right">規範</em>"#
        );
    }

    #[test]
    fn emphasis_bold_leaf_emits_b_tag() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let text = alloc.content_plain("重要");
        let n = alloc.emphasis(EmphasisKind::Bold, text, false);
        assert_eq!(
            render_node_to_string(n),
            r#"<b class="aozora-bold">重要</b>"#
        );
    }

    #[test]
    fn emphasis_italic_leaf_emits_i_tag() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let text = alloc.content_plain("e");
        let n = alloc.emphasis(EmphasisKind::Italic, text, false);
        assert_eq!(
            render_node_to_string(n),
            r#"<i class="aozora-italic">e</i>"#
        );
    }

    #[test]
    fn bold_container_emits_b_tag_open_and_close() {
        let arena = Arena::new();
        let alloc = BorrowedAllocator::new(&arena);
        let n = alloc.container(Container {
            kind: ContainerKind::Bold { block: false },
        });
        let mut open = String::new();
        render(n, true, &mut open).unwrap();
        let mut close = String::new();
        render(n, false, &mut close).unwrap();
        assert_eq!(open, r#"<b class="aozora-bold">"#);
        assert_eq!(close, "</b>");
    }

    #[test]
    fn italic_block_container_uses_block_div() {
        // The ここから-block form wraps paragraphs, so it renders a block
        // `<div>` (not the inline `<i>` the bare-range form uses) to keep
        // the `<div><p>…</p></div>` nesting valid.
        let arena = Arena::new();
        let alloc = BorrowedAllocator::new(&arena);
        let n = alloc.container(Container {
            kind: ContainerKind::Italic { block: true },
        });
        let mut open = String::new();
        render(n, true, &mut open).unwrap();
        let mut close = String::new();
        render(n, false, &mut close).unwrap();
        assert_eq!(
            open,
            r#"<div class="aozora-container aozora-container-italic">"#
        );
        assert_eq!(close, "</div>");
    }

    #[test]
    fn sashie_emits_figure_with_img() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let n = alloc.sashie("cover.png", None, None, None);
        assert_eq!(
            render_node_to_string(n),
            r#"<figure class="aozora-illustration"><img src="cover.png" alt="" /></figure>"#
        );
    }

    #[test]
    fn sashie_dimensions_emit_width_and_height() {
        // `横W×縦H` rides in `dimensions`; src stays a clean path and the
        // pixel size lands as `width` / `height` attributes.
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let n = alloc.sashie("fig42_03.png", None, Some("横480×縦640"), None);
        assert_eq!(
            render_node_to_string(n),
            r#"<figure class="aozora-illustration"><img src="fig42_03.png" width="480" height="640" alt="" /></figure>"#
        );
    }

    #[test]
    fn aozora_heading_large_emits_h1() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let text = alloc.content_plain("第一章");
        let n = alloc.aozora_heading(HeadingKind::Large, HeadingStyle::Standard, text);
        assert_eq!(
            render_node_to_string(n),
            r#"<h1 class="aozora-heading aozora-heading-large">第一章</h1>"#
        );
    }

    #[test]
    fn aozora_heading_window_medium_emits_styled_div() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let text = alloc.content_plain("見出し");
        let n = alloc.aozora_heading(HeadingKind::Medium, HeadingStyle::Window, text);
        assert_eq!(
            render_node_to_string(n),
            r#"<div class="aozora-heading aozora-heading-medium aozora-heading-window">見出し</div>"#
        );
    }

    #[test]
    fn wrap_indent_container_adds_wrap_class_and_attr() {
        let arena = Arena::new();
        let alloc = BorrowedAllocator::new(&arena);
        let n = alloc.container(Container {
            kind: ContainerKind::Indent {
                amount: 2,
                wrap: Some(4),
                center: false,
            },
        });
        let mut open = String::new();
        render(n, true, &mut open).unwrap();
        assert!(
            open.contains(
                "aozora-container-indent aozora-container-indent-2 aozora-container-wrap-indent"
            ),
            "{open}"
        );
        assert!(open.contains(r#"data-amount="2""#), "{open}");
        assert!(open.contains(r#"data-wrap="4""#), "{open}");
    }

    #[test]
    fn indent_emits_marker_with_amount_attr() {
        let arena = Arena::new();
        let alloc = BorrowedAllocator::new(&arena);
        let n = alloc.indent(Indent { amount: 2 });
        assert_eq!(
            render_node_to_string(n),
            r#"<span class="aozora-indent aozora-indent-2" data-amount="2"></span>"#
        );
    }

    #[test]
    fn align_end_zero_omits_numeric_class() {
        let arena = Arena::new();
        let alloc = BorrowedAllocator::new(&arena);
        let n = alloc.align_end(AlignEnd { offset: 0 });
        assert_eq!(
            render_node_to_string(n),
            r#"<span class="aozora-align-end" data-offset="0"></span>"#
        );
    }

    #[test]
    fn align_end_nonzero_offset_appends_numeric_class() {
        let arena = Arena::new();
        let alloc = BorrowedAllocator::new(&arena);
        let n = alloc.align_end(AlignEnd { offset: 2 });
        assert_eq!(
            render_node_to_string(n),
            r#"<span class="aozora-align-end aozora-align-end-2" data-offset="2"></span>"#
        );
    }

    #[test]
    fn section_break_kinds_use_stable_slugs() {
        let arena = Arena::new();
        let alloc = BorrowedAllocator::new(&arena);
        for (kind, slug) in [
            (SectionKind::Kaicho, "kaicho"),
            (SectionKind::Kaidan, "kaidan"),
            (SectionKind::Kaimihiraki, "kaimihiraki"),
        ] {
            let n = alloc.section_break(kind);
            assert_eq!(
                render_node_to_string(n),
                format!(r#"<div class="aozora-section-break aozora-section-break-{slug}"></div>"#),
            );
        }
    }

    #[test]
    fn container_open_close_round_trip() {
        let arena = Arena::new();
        let alloc = BorrowedAllocator::new(&arena);
        let n = alloc.container(Container {
            kind: ContainerKind::Indent {
                amount: 2,
                wrap: None,
                center: false,
            },
        });
        let mut open = String::new();
        render(n, true, &mut open).unwrap();
        let mut close = String::new();
        render(n, false, &mut close).unwrap();
        assert!(open.contains("aozora-container-indent aozora-container-indent-2"));
        assert!(open.contains(r#"data-amount="2""#));
        assert_eq!(close, "</div>");
    }

    #[test]
    fn inline_nodes_emit_nothing_on_exit() {
        let arena = Arena::new();
        let alloc = BorrowedAllocator::new(&arena);
        let n = alloc.page_break();
        let mut buf = String::new();
        render(n, false, &mut buf).expect("render exit pass never fails");
        assert!(buf.is_empty(), "PageBreak must emit nothing on exit");
    }

    // Helpers to render a container's open / close tag.
    fn open_tag(kind: ContainerKind) -> String {
        let arena = Arena::new();
        let alloc = BorrowedAllocator::new(&arena);
        let n = alloc.container(Container { kind });
        let mut out = String::new();
        render(n, true, &mut out).expect("container open render never fails");
        out
    }

    fn close_tag(kind: ContainerKind) -> String {
        let arena = Arena::new();
        let alloc = BorrowedAllocator::new(&arena);
        let n = alloc.container(Container { kind });
        let mut out = String::new();
        render(n, false, &mut out).expect("container close render never fails");
        out
    }

    // -------------------------------------------------------------------
    // Bouten: every BoutenKind maps to its stable slug; left position too.
    // -------------------------------------------------------------------

    #[test]
    fn bouten_every_kind_uses_its_slug() {
        for (kind, slug) in [
            (BoutenKind::Goma, "goma"),
            (BoutenKind::WhiteSesame, "shirogoma"),
            (BoutenKind::Circle, "maru"),
            (BoutenKind::WhiteCircle, "shiromaru"),
            (BoutenKind::DoubleCircle, "nijumaru"),
            (BoutenKind::Janome, "janome"),
            (BoutenKind::Cross, "batsu"),
            (BoutenKind::WhiteTriangle, "shirosankaku"),
            (BoutenKind::WavyLine, "namisen"),
            (BoutenKind::UnderLine, "bosen"),
            (BoutenKind::DoubleUnderLine, "nijubosen"),
            (BoutenKind::ChainLine, "kusarisen"),
            (BoutenKind::DashedLine, "hasen"),
            (BoutenKind::BlackTriangle, "kurosankaku"),
        ] {
            let arena = Arena::new();
            let mut alloc = BorrowedAllocator::new(&arena);
            let target = alloc.content_plain("対象");
            let n = alloc.bouten(kind, target, BoutenPosition::Right, false);
            assert_eq!(
                render_node_to_string(n),
                format!(
                    r#"<em class="aozora-bouten aozora-bouten-{slug} aozora-bouten-right">対象</em>"#
                ),
                "bouten slug mismatch for {kind:?}"
            );
        }
    }

    #[test]
    fn bouten_left_position_uses_left_slug() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let target = alloc.content_plain("対象");
        let n = alloc.bouten(BoutenKind::Goma, target, BoutenPosition::Left, false);
        assert_eq!(
            render_node_to_string(n),
            r#"<em class="aozora-bouten aozora-bouten-goma aozora-bouten-left">対象</em>"#
        );
    }

    // -------------------------------------------------------------------
    // Emphasis leaf: each semantic element + the dynamic FontSize span.
    // -------------------------------------------------------------------

    #[test]
    fn emphasis_super_and_sub_script_use_sup_sub() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let exponent = alloc.content_plain("2");
        let sup = alloc.emphasis(EmphasisKind::SuperScript, exponent, false);
        assert_eq!(
            render_node_to_string(sup),
            r#"<sup class="aozora-superscript">2</sup>"#
        );
        let index = alloc.content_plain("3");
        let sub = alloc.emphasis(EmphasisKind::SubScript, index, false);
        assert_eq!(
            render_node_to_string(sub),
            r#"<sub class="aozora-subscript">3</sub>"#
        );
    }

    #[test]
    fn emphasis_span_forms_use_span_with_slug() {
        for (kind, slug) in [
            (EmphasisKind::SmallRight, "kogaki-right"),
            (EmphasisKind::SmallLeft, "kogaki-left"),
            (EmphasisKind::KeigakomiInline, "keigakomi-inline"),
            (EmphasisKind::HorizontalInline, "horizontal"),
            (EmphasisKind::Caption, "caption"),
        ] {
            let arena = Arena::new();
            let mut alloc = BorrowedAllocator::new(&arena);
            let text = alloc.content_plain("X");
            let n = alloc.emphasis(kind, text, false);
            assert_eq!(
                render_node_to_string(n),
                format!(r#"<span class="aozora-{slug}">X</span>"#),
                "emphasis span slug mismatch for {kind:?}"
            );
        }
    }

    #[test]
    fn emphasis_font_size_positive_emits_larger_span() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let text = alloc.content_plain("大");
        let n = alloc.emphasis(EmphasisKind::FontSize { steps: 3 }, text, false);
        assert_eq!(
            render_node_to_string(n),
            r#"<span class="aozora-font-larger" data-steps="3">大</span>"#
        );
    }

    #[test]
    fn emphasis_font_size_negative_emits_smaller_span() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let text = alloc.content_plain("小");
        let n = alloc.emphasis(EmphasisKind::FontSize { steps: -2 }, text, false);
        assert_eq!(
            render_node_to_string(n),
            r#"<span class="aozora-font-smaller" data-steps="2">小</span>"#
        );
    }

    // -------------------------------------------------------------------
    // CombineUpright leaf + MarginNote + AngleQuote + kaeriten.
    // -------------------------------------------------------------------

    #[test]
    fn tate_chu_yoko_leaf_wraps_in_tcy_span() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let text = alloc.content_plain("12");
        let n = alloc.tate_chu_yoko(text, false);
        assert_eq!(
            render_node_to_string(n),
            r#"<span class="aozora-combine-upright">12</span>"#
        );
    }

    #[test]
    fn side_note_uses_ruby_box_with_sidenote_class() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let base = alloc.content_plain("底本");
        let note = alloc.content_plain("注記");
        let n = alloc.side_note(aozora_syntax::MarginNoteKind::Gloss, base, note);
        assert_eq!(
            render_node_to_string(n),
            r#"<ruby>底本<rp>(</rp><rt class="aozora-margin-note">注記</rt><rp>)</rp></ruby>"#
        );
    }

    #[test]
    fn left_ruby_marks_rt_with_left_class() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let base = alloc.content_plain("再読");
        let reading = alloc.content_plain("さい");
        let n = alloc.left_ruby(base, reading);
        assert_eq!(
            render_node_to_string(n),
            r#"<ruby>再読<rp>(</rp><rt class="aozora-ruby-left">さい</rt><rp>)</rp></ruby>"#
        );
    }

    #[test]
    fn angle_quote_wraps_display_glyphs() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let content = alloc.content_plain("引用");
        let n = alloc.angle_quote(content);
        assert_eq!(
            render_node_to_string(n),
            r#"<span class="aozora-angle-quote">《引用》</span>"#
        );
    }

    #[test]
    fn kaeriten_wraps_mark_and_escapes() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let n = alloc.kaeriten("レ");
        assert_eq!(
            render_node_to_string(n),
            r#"<sup class="aozora-kaeriten">レ</sup>"#
        );
    }

    // -------------------------------------------------------------------
    // Gaiji: resolved single / multi scalar + description fallback.
    // -------------------------------------------------------------------

    #[test]
    fn gaiji_resolved_char_emits_single_codepoint() {
        use aozora_encoding::gaiji::Resolved;
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let g = alloc.make_gaiji("desc", Some(Resolved::Char('枘')), None, false);
        let n = alloc.gaiji(g);
        assert_eq!(
            render_node_to_string(n),
            r#"<span class="aozora-gaiji" data-codepoint="U+6798">枘</span>"#
        );
    }

    #[test]
    fn gaiji_resolved_multi_lists_each_scalar() {
        use aozora_encoding::gaiji::Resolved;
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        // A combining sequence: U+304B U+309A.
        let g = alloc.make_gaiji(
            "desc",
            Some(Resolved::Multi("\u{304B}\u{309A}")),
            None,
            false,
        );
        let n = alloc.gaiji(g);
        assert_eq!(
            render_node_to_string(n),
            "<span class=\"aozora-gaiji\" data-codepoint=\"U+304B U+309A\">\u{304B}\u{309A}</span>"
        );
    }

    #[test]
    fn gaiji_unresolved_falls_back_to_description() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let g = alloc.make_gaiji("第3水準", None, None, false);
        let n = alloc.gaiji(g);
        assert_eq!(
            render_node_to_string(n),
            r#"<span class="aozora-gaiji" data-description="第3水準">第3水準</span>"#
        );
    }

    #[test]
    fn gaiji_unresolved_escapes_description_in_both_slots() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let g = alloc.make_gaiji("a<b>&", None, None, false);
        let n = alloc.gaiji(g);
        assert_eq!(
            render_node_to_string(n),
            r#"<span class="aozora-gaiji" data-description="a&lt;b&gt;&amp;">a&lt;b&gt;&amp;</span>"#
        );
    }

    // -------------------------------------------------------------------
    // Directive: warichu open/close shortcuts + the hidden-span default.
    // -------------------------------------------------------------------

    #[test]
    fn annotation_warichu_open_and_close_emit_span_pair() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let open = alloc.make_directive("［＃割り注］", DirectiveKind::WarichuOpen);
        let open_n = alloc.annotation(open);
        assert_eq!(
            render_node_to_string(open_n),
            r#"<span class="aozora-warichu">"#
        );
        let close = alloc.make_directive("［＃割り注終わり］", DirectiveKind::WarichuClose);
        let close_n = alloc.annotation(close);
        assert_eq!(render_node_to_string(close_n), "</span>");
    }

    #[test]
    fn annotation_hidden_span_escapes_raw() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let payload = alloc.make_directive("a<b>", DirectiveKind::Sic);
        let n = alloc.annotation(payload);
        assert_eq!(
            render_node_to_string(n),
            r#"<span class="aozora-directive" hidden>a&lt;b&gt;</span>"#
        );
    }

    // -------------------------------------------------------------------
    // Center + section break (every kind).
    // -------------------------------------------------------------------

    #[test]
    fn center_emits_zero_width_hook_for_both_forms() {
        let arena = Arena::new();
        let alloc = BorrowedAllocator::new(&arena);
        for page in [true, false] {
            let n = alloc.center(aozora_syntax::Center { page });
            assert_eq!(
                render_node_to_string(n),
                r#"<span class="aozora-center"></span>"#,
                "center hook differs for page={page}"
            );
        }
    }

    // -------------------------------------------------------------------
    // Illustration: general image form (description → alt) + caption.
    // -------------------------------------------------------------------

    #[test]
    fn sashie_general_form_puts_description_in_alt() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let n = alloc.sashie_general("map.png", "地図", None);
        assert_eq!(
            render_node_to_string(n),
            r#"<figure class="aozora-illustration"><img src="map.png" alt="地図" /></figure>"#
        );
    }

    #[test]
    fn sashie_caption_renders_figcaption() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let caption = alloc.content_plain("図の説明");
        let n = alloc.sashie("fig.png", Some("1"), None, Some(caption));
        assert_eq!(
            render_node_to_string(n),
            r#"<figure class="aozora-illustration"><img src="fig.png" alt="" /><figcaption>図の説明</figcaption></figure>"#
        );
    }

    #[test]
    fn sashie_malformed_dimensions_drop_size_attrs() {
        // `parse_sashie_dimensions` returns None for a non `横W×縦H` shape.
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let n = alloc.sashie("fig.png", None, Some("不明"), None);
        assert_eq!(
            render_node_to_string(n),
            r#"<figure class="aozora-illustration"><img src="fig.png" alt="" /></figure>"#
        );
    }

    // -------------------------------------------------------------------
    // Heading leaf — small level + same-line style.
    // -------------------------------------------------------------------

    #[test]
    fn aozora_heading_small_same_line_emits_h3_with_modifier() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let text = alloc.content_plain("見出し");
        let n = alloc.aozora_heading(HeadingKind::Small, HeadingStyle::SameLine, text);
        assert_eq!(
            render_node_to_string(n),
            r#"<h3 class="aozora-heading aozora-heading-small aozora-heading-same-line">見出し</h3>"#
        );
    }

    // -------------------------------------------------------------------
    // HeadingHint — standard (no data-style) vs styled.
    // -------------------------------------------------------------------

    #[test]
    fn heading_hint_standard_omits_data_style() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let n = alloc.heading_hint(1, HeadingStyle::Standard, "対象");
        assert_eq!(
            render_node_to_string(n),
            r#"<span class="aozora-heading-hint" data-level="1" data-target="対象" hidden></span>"#
        );
    }

    #[test]
    fn heading_hint_styled_includes_data_style_and_escapes_target() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let n = alloc.heading_hint(2, HeadingStyle::Window, "a<b>");
        assert_eq!(
            render_node_to_string(n),
            r#"<span class="aozora-heading-hint" data-level="2" data-style="window" data-target="a&lt;b&gt;" hidden></span>"#
        );
    }

    // -------------------------------------------------------------------
    // Container open: one assertion per ContainerKind family / branch.
    // -------------------------------------------------------------------

    #[test]
    fn container_indent_center_adds_center_class() {
        assert_eq!(
            open_tag(ContainerKind::Indent {
                amount: 2,
                wrap: None,
                center: true,
            }),
            r#"<div class="aozora-container aozora-container-indent aozora-container-indent-2 aozora-container-center" data-amount="2">"#
        );
    }

    #[test]
    fn container_align_end_open_carries_offset() {
        assert_eq!(
            open_tag(ContainerKind::AlignEnd { offset: 3 }),
            r#"<div class="aozora-container aozora-container-align-end" data-offset="3">"#
        );
        assert_eq!(close_tag(ContainerKind::AlignEnd { offset: 3 }), "</div>");
    }

    #[test]
    fn container_line_width_open_carries_width() {
        assert_eq!(
            open_tag(ContainerKind::LineWidth { width: 20 }),
            r#"<div class="aozora-container aozora-container-line-width" data-width="20">"#
        );
        assert_eq!(close_tag(ContainerKind::LineWidth { width: 20 }), "</div>");
    }

    #[test]
    fn container_keigakomi_and_warichu_open_close() {
        assert_eq!(
            open_tag(ContainerKind::Framed),
            r#"<div class="aozora-container aozora-container-keigakomi">"#
        );
        assert_eq!(close_tag(ContainerKind::Framed), "</div>");
        assert_eq!(
            open_tag(ContainerKind::Warichu),
            r#"<div class="aozora-container aozora-container-warichu">"#
        );
        assert_eq!(close_tag(ContainerKind::Warichu), "</div>");
    }

    #[test]
    fn container_bouten_range_uses_em_with_slugs() {
        let kind = ContainerKind::BoutenRange {
            kind: BoutenKind::UnderLine,
            position: BoutenPosition::Left,
        };
        assert_eq!(
            open_tag(kind),
            r#"<em class="aozora-bouten aozora-bouten-bosen aozora-bouten-left">"#
        );
        assert_eq!(close_tag(kind), "</em>");
    }

    #[test]
    fn container_bold_block_uses_div() {
        assert_eq!(
            open_tag(ContainerKind::Bold { block: true }),
            r#"<div class="aozora-container aozora-container-bold">"#
        );
        assert_eq!(close_tag(ContainerKind::Bold { block: true }), "</div>");
    }

    #[test]
    fn container_italic_bare_range_uses_i() {
        assert_eq!(
            open_tag(ContainerKind::Italic { block: false }),
            r#"<i class="aozora-italic">"#
        );
        assert_eq!(close_tag(ContainerKind::Italic { block: false }), "</i>");
    }

    #[test]
    fn container_columns_carries_count() {
        assert_eq!(
            open_tag(ContainerKind::Columns { count: 2 }),
            r#"<div class="aozora-container aozora-container-columns" data-columns="2">"#
        );
        assert_eq!(close_tag(ContainerKind::Columns { count: 2 }), "</div>");
    }

    #[test]
    fn container_table_and_horizontal() {
        assert_eq!(
            open_tag(ContainerKind::Table),
            r#"<div class="aozora-container aozora-container-table">"#
        );
        assert_eq!(close_tag(ContainerKind::Table), "</div>");
        assert_eq!(
            open_tag(ContainerKind::Horizontal),
            r#"<div class="aozora-container aozora-container-horizontal">"#
        );
        assert_eq!(close_tag(ContainerKind::Horizontal), "</div>");
    }

    #[test]
    fn container_font_size_positive_and_negative() {
        assert_eq!(
            open_tag(ContainerKind::FontSize { steps: 3 }),
            r#"<div class="aozora-container aozora-container-font-larger" data-steps="3">"#
        );
        assert_eq!(close_tag(ContainerKind::FontSize { steps: 3 }), "</div>");
        assert_eq!(
            open_tag(ContainerKind::FontSize { steps: -2 }),
            r#"<div class="aozora-container aozora-container-font-smaller" data-steps="2">"#
        );
        assert_eq!(close_tag(ContainerKind::FontSize { steps: -2 }), "</div>");
    }

    #[test]
    fn container_heading_block_uses_heading_element() {
        let kind = ContainerKind::Heading {
            kind: HeadingKind::Large,
            style: HeadingStyle::Standard,
            block: true,
        };
        assert_eq!(
            open_tag(kind),
            r#"<h1 class="aozora-heading aozora-heading-large">"#
        );
        assert_eq!(close_tag(kind), "</h1>");
    }

    #[test]
    fn container_heading_window_uses_div_element() {
        let kind = ContainerKind::Heading {
            kind: HeadingKind::Medium,
            style: HeadingStyle::Window,
            block: false,
        };
        assert_eq!(
            open_tag(kind),
            r#"<div class="aozora-heading aozora-heading-medium aozora-heading-window">"#
        );
        assert_eq!(close_tag(kind), "</div>");
    }

    #[test]
    fn container_small_script_left_and_right() {
        assert_eq!(
            open_tag(ContainerKind::SmallScript {
                side: BoutenPosition::Left,
            }),
            r#"<span class="aozora-kogaki-left">"#
        );
        assert_eq!(
            open_tag(ContainerKind::SmallScript {
                side: BoutenPosition::Right,
            }),
            r#"<span class="aozora-kogaki-right">"#
        );
        assert_eq!(
            close_tag(ContainerKind::SmallScript {
                side: BoutenPosition::Right,
            }),
            "</span>"
        );
    }

    #[test]
    fn container_caption_range_and_block() {
        assert_eq!(
            open_tag(ContainerKind::Caption { block: false }),
            r#"<span class="aozora-caption">"#
        );
        assert_eq!(
            close_tag(ContainerKind::Caption { block: false }),
            "</span>"
        );
        assert_eq!(
            open_tag(ContainerKind::Caption { block: true }),
            r#"<div class="aozora-container aozora-caption">"#
        );
        assert_eq!(close_tag(ContainerKind::Caption { block: true }), "</div>");
    }

    #[test]
    fn container_tcy_range_uses_tcy_span() {
        assert_eq!(
            open_tag(ContainerKind::CombineUprightRange),
            r#"<span class="aozora-combine-upright">"#
        );
        assert_eq!(close_tag(ContainerKind::CombineUprightRange), "</span>");
    }

    #[test]
    fn render_content_walks_text_gaiji_and_annotation_segments() {
        // A ruby base built from mixed segments exercises every arm of
        // `render_content`: Text (escaped), Gaiji (nested render), and
        // Directive (hidden span).
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let g = alloc.make_gaiji("外字", None, None, false);
        let seg_g = alloc.seg_gaiji(g);
        let ann = alloc.make_directive("［＃注］", DirectiveKind::Unknown);
        let seg_a = alloc.seg_annotation(ann);
        let seg_t = alloc.seg_text("前<");
        let base = alloc.content_segments(&[seg_t, seg_g, seg_a]);
        let reading = alloc.content_plain("よ");
        let n = alloc.ruby(base, reading, true);
        assert_eq!(
            render_node_to_string(n),
            concat!(
                "<ruby>前&lt;",
                r#"<span class="aozora-gaiji" data-description="外字">外字</span>"#,
                r#"<span class="aozora-directive" hidden>［＃注］</span>"#,
                "<rp>(</rp><rt>よ</rt><rp>)</rp></ruby>"
            )
        );
    }
}
