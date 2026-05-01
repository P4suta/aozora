//! HTML rendering for individual borrowed-AST nodes.
//!
//! Per-node renderer parameterised over the source/arena lifetime
//! `'src`. Public entry point: [`render`].

use core::fmt::{self, Write};

use aozora_syntax::borrowed::{
    Annotation, AozoraNode, Bouten, Content, DoubleRuby, Gaiji, Kaeriten, Ruby, Segment,
};
use aozora_syntax::{AlignEnd, AnnotationKind, Container, ContainerKind, Indent, SectionKind};

use crate::bouten;

/// Render a single borrowed [`AozoraNode`] into `writer`.
///
/// `entering` follows the standard tree-walker enter/exit convention:
/// inline / leaf nodes emit their markup only on `entering == true`
/// and produce nothing on the exit pass. Container nodes
/// ([`AozoraNode::Container`]) emit an opening tag on enter and a
/// closing tag on exit — the calling block walker drives children
/// between the two events.
///
/// # Errors
///
/// Propagates formatter write errors.
pub fn render<W: Write>(node: AozoraNode<'_>, entering: bool, writer: &mut W) -> fmt::Result {
    match node {
        AozoraNode::Container(c) => render_container(c, entering, writer),
        _ if !entering => Ok(()),
        AozoraNode::Ruby(r) => render_ruby(r, writer),
        AozoraNode::Bouten(b) => render_bouten(b, writer),
        AozoraNode::TateChuYoko(t) => {
            writer.write_str(r#"<span class="aozora-tcy">"#)?;
            render_content(t.text.get(), writer)?;
            writer.write_str("</span>")
        }
        AozoraNode::Gaiji(g) => render_gaiji(g, writer),
        AozoraNode::Indent(i) => render_indent(i, writer),
        AozoraNode::AlignEnd(a) => render_align_end(a, writer),
        AozoraNode::PageBreak => writer.write_str(r#"<div class="aozora-page-break"></div>"#),
        AozoraNode::SectionBreak(k) => {
            let slug = match k {
                SectionKind::Choho => "choho",
                SectionKind::Dan => "dan",
                SectionKind::Spread => "spread",
                _ => "other",
            };
            write!(
                writer,
                r#"<div class="aozora-section-break aozora-section-break-{slug}"></div>"#,
            )
        }
        AozoraNode::Annotation(a) => render_annotation(a, writer),
        AozoraNode::Kaeriten(k) => render_kaeriten(k, writer),
        AozoraNode::DoubleRuby(d) => render_double_ruby(d, writer),
        // Other variants — emit a fallback comment so the rendered
        // HTML stays diagnosable. Mirrors the owned renderer's
        // catch-all behavior for AozoraHeading / HeadingHint / Sashie /
        // Warichu / Keigakomi (which the legacy renderer also routes
        // through `fallback`).
        _ => fallback(node, writer),
    }
}

fn render_ruby<W: Write>(r: &Ruby<'_>, writer: &mut W) -> fmt::Result {
    writer.write_str("<ruby>")?;
    render_content(r.base.get(), writer)?;
    writer.write_str("<rp>(</rp><rt>")?;
    render_content(r.reading.get(), writer)?;
    writer.write_str("</rt><rp>)</rp></ruby>")
}

fn render_bouten<W: Write>(b: &Bouten<'_>, writer: &mut W) -> fmt::Result {
    write!(
        writer,
        r#"<em class="aozora-bouten aozora-bouten-{kind} aozora-bouten-{pos}">"#,
        kind = bouten::kind_slug(b.kind),
        pos = bouten::position_slug(b.position),
    )?;
    render_content(b.target.get(), writer)?;
    writer.write_str("</em>")
}

/// Render a [`Content`] by walking its segments in order.
fn render_content<W: Write>(content: Content<'_>, writer: &mut W) -> fmt::Result {
    for seg in content {
        match seg {
            Segment::Text(t) => escape_text(t, writer)?,
            Segment::Gaiji(g) => render_gaiji(g, writer)?,
            Segment::Annotation(a) => render_annotation(a, writer)?,
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

fn render_annotation<W: Write>(a: &Annotation<'_>, writer: &mut W) -> fmt::Result {
    match a.kind {
        AnnotationKind::WarichuOpen => return writer.write_str(r#"<span class="aozora-warichu">"#),
        AnnotationKind::WarichuClose => return writer.write_str("</span>"),
        _ => {}
    }
    writer.write_str(r#"<span class="aozora-annotation" hidden>"#)?;
    escape_text(a.raw, writer)?;
    writer.write_str("</span>")
}

fn render_kaeriten<W: Write>(k: &Kaeriten<'_>, writer: &mut W) -> fmt::Result {
    writer.write_str(r#"<sup class="aozora-kaeriten">"#)?;
    escape_text(k.mark, writer)?;
    writer.write_str("</sup>")
}

fn render_container<W: Write>(c: Container, entering: bool, writer: &mut W) -> fmt::Result {
    if entering {
        match c.kind {
            ContainerKind::Indent { amount } => {
                write!(
                    writer,
                    r#"<div class="aozora-container aozora-container-indent aozora-container-indent-{amount}" data-amount="{amount}">"#,
                )
            }
            ContainerKind::AlignEnd { offset } => {
                write!(
                    writer,
                    r#"<div class="aozora-container aozora-container-align-end" data-offset="{offset}">"#,
                )
            }
            ContainerKind::Keigakomi => {
                writer.write_str(r#"<div class="aozora-container aozora-container-keigakomi">"#)
            }
            ContainerKind::Warichu => {
                writer.write_str(r#"<div class="aozora-container aozora-container-warichu">"#)
            }
            _ => writer.write_str(r#"<div class="aozora-container">"#),
        }
    } else {
        writer.write_str("</div>")
    }
}

fn render_double_ruby<W: Write>(d: &DoubleRuby<'_>, writer: &mut W) -> fmt::Result {
    writer.write_str(r#"<span class="aozora-double-ruby">≪"#)?;
    render_content(d.content, writer)?;
    writer.write_str("≫</span>")
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

fn fallback<W: Write>(node: AozoraNode<'_>, writer: &mut W) -> fmt::Result {
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
    use aozora_syntax::borrowed::{AozoraNode, Arena};
    use aozora_syntax::{
        AlignEnd, AnnotationKind, BoutenKind, BoutenPosition, Indent, SectionKind,
    };

    fn render_node_to_string(node: AozoraNode<'_>) -> String {
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
        let payload = alloc.make_annotation("［＃改ページ］", AnnotationKind::Unknown);
        let n = alloc.annotation(payload);
        assert_eq!(
            render_node_to_string(n),
            r#"<span class="aozora-annotation" hidden>［＃改ページ］</span>"#
        );
    }

    #[test]
    fn bouten_kind_and_position_slug() {
        let arena = Arena::new();
        let mut alloc = BorrowedAllocator::new(&arena);
        let target = alloc.content_plain("可哀想");
        let n = alloc.bouten(BoutenKind::Goma, target, BoutenPosition::Right);
        assert_eq!(
            render_node_to_string(n),
            r#"<em class="aozora-bouten aozora-bouten-goma aozora-bouten-right">可哀想</em>"#
        );
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
            (SectionKind::Choho, "choho"),
            (SectionKind::Dan, "dan"),
            (SectionKind::Spread, "spread"),
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
            kind: ContainerKind::Indent { amount: 2 },
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
        render(n, false, &mut buf).unwrap();
        assert!(buf.is_empty(), "PageBreak must emit nothing on exit");
    }
}
