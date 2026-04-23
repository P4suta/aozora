//! HTML rendering for Aozora AST nodes.
//!
//! Emits semantic HTML5; visual styling comes from the paired
//! `afm-horizontal.css` / `afm-vertical.css` stylesheets. This module keeps
//! rendering decisions centralised so the comrak-side adapter is a thin
//! dispatcher.
//!
//! Public entry point: [`render`].

use core::fmt::Write;

use afm_syntax::{AlignEnd, AozoraNode, Bouten, Indent, Ruby, SectionKind};

use crate::aozora::bouten;

/// Render a single [`AozoraNode`] into `writer`. Called from the comrak fork's
/// `NodeValue::Aozora(_)` renderer arm, which passes `&mut dyn fmt::Write` over
/// its own output sink.
///
/// # Errors
///
/// Propagates formatter write errors.
pub fn render(node: &AozoraNode, writer: &mut dyn Write) -> core::fmt::Result {
    match node {
        AozoraNode::Ruby(r) => render_ruby(r, writer),
        AozoraNode::Bouten(b) => render_bouten(b, writer),
        AozoraNode::TateChuYoko(t) => {
            writer.write_str(r#"<span class="afm-tcy">"#)?;
            escape_text(&t.text, writer)?;
            writer.write_str("</span>")
        }
        AozoraNode::Gaiji(g) => {
            writer.write_str(r#"<span class="afm-gaiji">"#)?;
            if let Some(c) = g.ucs {
                let mut buf = [0u8; 4];
                writer.write_str(c.encode_utf8(&mut buf))?;
            } else {
                escape_text(&g.description, writer)?;
            }
            writer.write_str("</span>")
        }
        AozoraNode::Indent(i) => render_indent(*i, writer),
        AozoraNode::AlignEnd(a) => render_align_end(*a, writer),
        AozoraNode::PageBreak => writer.write_str(r#"<div class="afm-page-break"></div>"#),
        AozoraNode::SectionBreak(k) => {
            let slug = match k {
                SectionKind::Choho => "choho",
                SectionKind::Dan => "dan",
                SectionKind::Spread => "spread",
                _ => "other",
            };
            write!(
                writer,
                r#"<div class="afm-section-break afm-section-break-{slug}"></div>"#,
            )
        }
        AozoraNode::Annotation(a) => {
            // Round-trip preservation: visible-but-unstyled by default, carrying
            // the raw annotation text as accessible content. Rendered verbatim
            // inside an HTML comment so CommonMark/GFM-only readers don't see it,
            // and as a span for accessibility on the visible path.
            writer.write_str(r#"<span class="afm-annotation" hidden>"#)?;
            escape_text(&a.raw, writer)?;
            writer.write_str("</span>")
        }
        // Block / container kinds — ruby, bouten, etc. may gain distinct markup
        // in M1; for M0 we emit a class-carrying wrapper so presence is visible.
        _ => fallback(node, writer),
    }
}

fn render_ruby(r: &Ruby, writer: &mut dyn Write) -> core::fmt::Result {
    writer.write_str("<ruby>")?;
    escape_text(&r.base, writer)?;
    writer.write_str("<rp>(</rp><rt>")?;
    escape_text(&r.reading, writer)?;
    writer.write_str("</rt><rp>)</rp></ruby>")
}

/// Forward-reference bouten renders as a semantic `<em>` wrapping the
/// annotated literal, with a per-kind class for CSS styling. The preceding
/// plain occurrence of the literal remains in the surrounding text stream;
/// visual deduplication (hiding the plain copy so the bouten-marked run
/// takes its place) is a stylesheet concern — see
/// `crates/afm-book/theme/afm-horizontal.css` for the CSS class contract.
fn render_bouten(b: &Bouten, writer: &mut dyn Write) -> core::fmt::Result {
    write!(
        writer,
        r#"<em class="afm-bouten afm-bouten-{slug}">"#,
        slug = bouten::kind_slug(b.kind),
    )?;
    escape_text(&b.target, writer)?;
    writer.write_str("</em>")
}

/// Leaf `{N}字下げ` — emits an empty marker `<span>` with a per-amount
/// class. The annotation applies to the following inline run; the
/// stylesheet uses sibling selectors to apply the indent. Rendering as
/// `<span>` (not `<div>`) keeps the markup valid inside `<p>`, which is
/// where comrak places the inline-hook result.
fn render_indent(i: Indent, writer: &mut dyn Write) -> core::fmt::Result {
    write!(
        writer,
        r#"<span class="afm-indent afm-indent-{n}" data-amount="{n}"></span>"#,
        n = i.amount,
    )
}

/// Leaf `地付き` (offset 0) / `地からN字上げ` (offset N). Same shape as
/// [`render_indent`]: an empty marker span that the stylesheet turns into
/// a right-aligned block.
fn render_align_end(a: AlignEnd, writer: &mut dyn Write) -> core::fmt::Result {
    if a.offset == 0 {
        writer.write_str(r#"<span class="afm-align-end" data-offset="0"></span>"#)
    } else {
        write!(
            writer,
            r#"<span class="afm-align-end afm-align-end-{n}" data-offset="{n}"></span>"#,
            n = a.offset,
        )
    }
}

fn fallback(node: &AozoraNode, writer: &mut dyn Write) -> core::fmt::Result {
    write!(writer, "<!-- {} -->", node.xml_node_name())
}

/// Minimal HTML5 text escape for the five structural characters.
fn escape_text(text: &str, writer: &mut dyn Write) -> core::fmt::Result {
    for ch in text.chars() {
        match ch {
            '&' => writer.write_str("&amp;")?,
            '<' => writer.write_str("&lt;")?,
            '>' => writer.write_str("&gt;")?,
            '"' => writer.write_str("&quot;")?,
            '\'' => writer.write_str("&#x27;")?,
            _ => writer.write_char(ch)?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use afm_syntax::{
        AlignEnd, Annotation, AnnotationKind, Bouten, BoutenKind, Indent, Ruby, TateChuYoko,
    };

    fn render_to_string(node: &AozoraNode) -> String {
        let mut out = String::new();
        render(node, &mut out).expect("fmt::Write into String never fails");
        out
    }

    #[test]
    fn ruby_emits_rp_rt_canonical_form() {
        let r = AozoraNode::Ruby(Ruby {
            base: "青梅".into(),
            reading: "おうめ".into(),
            delim_explicit: true,
        });
        assert_eq!(
            render_to_string(&r),
            "<ruby>青梅<rp>(</rp><rt>おうめ</rt><rp>)</rp></ruby>"
        );
    }

    #[test]
    fn ruby_escapes_structural_characters() {
        let r = AozoraNode::Ruby(Ruby {
            base: "<x>".into(),
            reading: "&y".into(),
            delim_explicit: true,
        });
        let out = render_to_string(&r);
        assert!(out.contains("&lt;x&gt;"));
        assert!(out.contains("&amp;y"));
    }

    #[test]
    fn tcy_wraps_in_afm_tcy_span() {
        let n = AozoraNode::TateChuYoko(TateChuYoko { text: "20".into() });
        assert_eq!(render_to_string(&n), r#"<span class="afm-tcy">20</span>"#);
    }

    #[test]
    fn page_break_is_self_contained_div() {
        assert_eq!(
            render_to_string(&AozoraNode::PageBreak),
            r#"<div class="afm-page-break"></div>"#
        );
    }

    #[test]
    fn annotation_is_hidden_round_trip() {
        let n = AozoraNode::Annotation(Annotation {
            raw: "［＃改ページ］".into(),
            kind: AnnotationKind::Unknown,
        });
        assert_eq!(
            render_to_string(&n),
            r#"<span class="afm-annotation" hidden>［＃改ページ］</span>"#
        );
    }

    #[test]
    fn bouten_emits_semantic_em_with_kind_slug() {
        let n = AozoraNode::Bouten(Bouten {
            kind: BoutenKind::Goma,
            target: "可哀想".into(),
        });
        assert_eq!(
            render_to_string(&n),
            r#"<em class="afm-bouten afm-bouten-goma">可哀想</em>"#
        );
    }

    #[test]
    fn bouten_escapes_structural_characters_in_target() {
        let n = AozoraNode::Bouten(Bouten {
            kind: BoutenKind::WavyLine,
            target: "a<b&c".into(),
        });
        assert_eq!(
            render_to_string(&n),
            r#"<em class="afm-bouten afm-bouten-wavy-line">a&lt;b&amp;c</em>"#
        );
    }

    #[test]
    fn indent_emits_empty_marker_span_with_amount_class() {
        let n = AozoraNode::Indent(Indent { amount: 2 });
        assert_eq!(
            render_to_string(&n),
            r#"<span class="afm-indent afm-indent-2" data-amount="2"></span>"#
        );
    }

    #[test]
    fn align_end_zero_offset_omits_numeric_class() {
        let n = AozoraNode::AlignEnd(AlignEnd { offset: 0 });
        assert_eq!(
            render_to_string(&n),
            r#"<span class="afm-align-end" data-offset="0"></span>"#
        );
    }

    #[test]
    fn align_end_nonzero_offset_appends_numeric_class() {
        let n = AozoraNode::AlignEnd(AlignEnd { offset: 2 });
        assert_eq!(
            render_to_string(&n),
            r#"<span class="afm-align-end afm-align-end-2" data-offset="2"></span>"#
        );
    }

    #[test]
    fn bouten_kind_slugs_are_stable_across_variants() {
        // Brittle on purpose — if a BoutenKind variant is renamed, the CSS
        // class contract breaks here, before reaching the stylesheet tests.
        for (kind, want_slug) in [
            (BoutenKind::Goma, "goma"),
            (BoutenKind::Circle, "circle"),
            (BoutenKind::WhiteCircle, "white-circle"),
            (BoutenKind::DoubleCircle, "double-circle"),
            (BoutenKind::Janome, "janome"),
            (BoutenKind::WavyLine, "wavy-line"),
            (BoutenKind::UnderLine, "under-line"),
        ] {
            let html = render_to_string(&AozoraNode::Bouten(Bouten {
                kind,
                target: "x".into(),
            }));
            let expected = format!(r#"<em class="afm-bouten afm-bouten-{want_slug}">x</em>"#);
            assert_eq!(html, expected, "kind={kind:?}");
        }
    }
}
