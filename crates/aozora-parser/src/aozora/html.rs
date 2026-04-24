//! HTML rendering for Aozora AST nodes.
//!
//! Emits semantic HTML5; visual styling comes from the paired
//! `afm-horizontal.css` / `afm-vertical.css` stylesheets. This module keeps
//! rendering decisions centralised so the comrak-side adapter is a thin
//! dispatcher.
//!
//! Public entry point: [`render`].

use core::fmt::{self, Write};

use afm_syntax::{
    AlignEnd, Annotation, AozoraNode, Bouten, Container, ContainerKind, Content, DoubleRuby, Gaiji,
    Indent, Kaeriten, Ruby, SectionKind, SegmentRef,
};

use crate::aozora::bouten;

/// Render a single [`AozoraNode`] into `writer`. Called from the
/// comrak fork's `NodeValue::Aozora(_)` renderer arm.
///
/// `entering` is comrak's standard enter/exit event flag. Leaf and
/// inline nodes emit their markup only on `entering == true` and
/// ignore the exit pass. Container-type nodes ([`AozoraNode::Container`],
/// the paired-block wrapper) emit an opening tag on enter and a
/// closing tag on exit, mirroring comrak's native `<ul>` / `<div>`
/// rendering contract so the children comrak walks between the two
/// events land inside the wrapper.
///
/// # Errors
///
/// Propagates formatter write errors.
pub fn render(node: &AozoraNode, entering: bool, writer: &mut dyn Write) -> fmt::Result {
    match node {
        AozoraNode::Container(c) => render_container(*c, entering, writer),
        _ if !entering => Ok(()),
        AozoraNode::Ruby(r) => render_ruby(r, writer),
        AozoraNode::Bouten(b) => render_bouten(b, writer),
        AozoraNode::TateChuYoko(t) => {
            writer.write_str(r#"<span class="afm-tcy">"#)?;
            render_content(&t.text, writer)?;
            writer.write_str("</span>")
        }
        AozoraNode::Gaiji(g) => render_gaiji(g, writer),
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
        AozoraNode::Annotation(a) => render_annotation(a, writer),
        AozoraNode::Kaeriten(k) => render_kaeriten(k, writer),
        AozoraNode::DoubleRuby(d) => render_double_ruby(d, writer),
        // Block / container kinds — ruby, bouten, etc. may gain distinct markup
        // in M1; for M0 we emit a class-carrying wrapper so presence is visible.
        _ => fallback(node, writer),
    }
}

fn render_ruby(r: &Ruby, writer: &mut dyn Write) -> fmt::Result {
    writer.write_str("<ruby>")?;
    render_content(&r.base, writer)?;
    writer.write_str("<rp>(</rp><rt>")?;
    render_content(&r.reading, writer)?;
    writer.write_str("</rt><rp>)</rp></ruby>")
}

/// Forward-reference bouten renders as a semantic `<em>` wrapping the
/// annotated literal, with a per-kind class for CSS styling and a
/// per-position modifier (`afm-bouten-right` / `afm-bouten-left`) so
/// the stylesheet can place the marks on either side of the base
/// text. The preceding plain occurrence of the literal remains in the
/// surrounding text stream; visual deduplication (hiding the plain
/// copy so the bouten-marked run takes its place) is a stylesheet
/// concern — see `crates/afm-book/theme/afm-horizontal.css` for the
/// CSS class contract.
fn render_bouten(b: &Bouten, writer: &mut dyn Write) -> fmt::Result {
    write!(
        writer,
        r#"<em class="afm-bouten afm-bouten-{kind} afm-bouten-{pos}">"#,
        kind = bouten::kind_slug(b.kind),
        pos = bouten::position_slug(b.position),
    )?;
    render_content(&b.target, writer)?;
    writer.write_str("</em>")
}

/// Render a [`Content`] by walking its segments in order. Plain content
/// follows the fast path (a single `escape_text` call via the iterator's
/// synthesised [`SegmentRef::Text`]); `Segments` dispatch per element.
///
/// Nested gaiji / annotations render with their outer wrapper markup —
/// `<rt>` accommodates child span elements per HTML5 content model,
/// so emitting `<span class="afm-gaiji">X</span>` inside a ruby
/// reading is well-formed. Same for `<em class="afm-bouten-*">`.
fn render_content(content: &Content, writer: &mut dyn Write) -> fmt::Result {
    for seg in content {
        match seg {
            SegmentRef::Text(t) => escape_text(t, writer)?,
            SegmentRef::Gaiji(g) => render_gaiji(g, writer)?,
            SegmentRef::Annotation(a) => render_annotation(a, writer)?,
            // `SegmentRef` is `#[non_exhaustive]` to allow future variants
            // (e.g. embedded bouten, ruby-in-ruby). Emit nothing for now;
            // once such a variant lands, this arm should be replaced with
            // a dedicated renderer.
            _ => {}
        }
    }
    Ok(())
}

fn render_gaiji(g: &Gaiji, writer: &mut dyn Write) -> fmt::Result {
    writer.write_str(r#"<span class="afm-gaiji">"#)?;
    if let Some(c) = g.ucs {
        let mut buf = [0u8; 4];
        writer.write_str(c.encode_utf8(&mut buf))?;
    } else {
        escape_text(&g.description, writer)?;
    }
    writer.write_str("</span>")
}

fn render_annotation(a: &Annotation, writer: &mut dyn Write) -> fmt::Result {
    use afm_syntax::AnnotationKind;
    // Inline warichu pair — `［＃割り注］X［＃割り注終わり］`. The Aozora
    // spec has deprecated the block form (`ここから割り注`…) in favour
    // of this inline shape, so we emit an opening `<span>` on
    // `WarichuOpen` and a closing `</span>` on `WarichuClose`. The
    // body text between them flows inline with the surrounding prose
    // rather than being wrapped in a block-level container.
    match a.kind {
        AnnotationKind::WarichuOpen => return writer.write_str(r#"<span class="afm-warichu">"#),
        AnnotationKind::WarichuClose => return writer.write_str("</span>"),
        _ => {}
    }
    // Round-trip preservation: visible-but-unstyled by default, carrying
    // the raw annotation text as accessible content, kept inside a
    // hidden span so CommonMark/GFM-only readers don't see it but
    // accessibility tools do.
    writer.write_str(r#"<span class="afm-annotation" hidden>"#)?;
    escape_text(&a.raw, writer)?;
    writer.write_str("</span>")
}

fn render_kaeriten(k: &Kaeriten, writer: &mut dyn Write) -> fmt::Result {
    // 返り点 as a small side-marker. `<sup>` is the natural semantic
    // vehicle for a superscript-like reading mark; the CSS theme can
    // tune size / position per writing mode.
    writer.write_str(r#"<sup class="afm-kaeriten">"#)?;
    escape_text(&k.mark, writer)?;
    writer.write_str("</sup>")
}

/// Render a paired block container. On enter, opens a `<div>` with
/// a per-kind class (and an optional numeric amount attribute for
/// the indent / align-end variants that carry a count); on exit,
/// closes the `</div>`. The intervening child blocks — paragraphs,
/// headings, nested containers — are walked by comrak's standard
/// tree renderer between the two calls.
///
/// The class-contract is pinned by `tests/block_structure_interaction.rs`
/// so stylesheet consumers can rely on the token list.
fn render_container(c: Container, entering: bool, writer: &mut dyn Write) -> fmt::Result {
    if entering {
        match c.kind {
            ContainerKind::Indent { amount } => {
                write!(
                    writer,
                    r#"<div class="afm-container afm-container-indent afm-container-indent-{amount}" data-amount="{amount}">"#,
                )
            }
            ContainerKind::AlignEnd { offset } => {
                write!(
                    writer,
                    r#"<div class="afm-container afm-container-align-end" data-offset="{offset}">"#,
                )
            }
            ContainerKind::Keigakomi => {
                writer.write_str(r#"<div class="afm-container afm-container-keigakomi">"#)
            }
            ContainerKind::Warichu => {
                writer.write_str(r#"<div class="afm-container afm-container-warichu">"#)
            }
            _ => writer.write_str(r#"<div class="afm-container">"#),
        }
    } else {
        writer.write_str("</div>")
    }
}

/// Render a `《《X》》` (double angle-bracket) span.
///
/// The Aozora annotation manual recommends disambiguating these
/// against single `《…》` ruby markers by emitting the academic
/// "double-angle quotation" characters U+226A (`≪`) and U+226B (`≫`)
/// around the payload, so the rendered text never collides visually
/// with ruby parentheses. A dedicated `afm-double-ruby` wrapper lets
/// the stylesheet tune size / spacing without the content markup
/// having to change per writing mode.
fn render_double_ruby(d: &DoubleRuby, writer: &mut dyn Write) -> fmt::Result {
    writer.write_str(r#"<span class="afm-double-ruby">≪"#)?;
    render_content(&d.content, writer)?;
    writer.write_str("≫</span>")
}

/// Leaf `{N}字下げ` — emits an empty marker `<span>` with a per-amount
/// class. The annotation applies to the following inline run; the
/// stylesheet uses sibling selectors to apply the indent. Rendering as
/// `<span>` (not `<div>`) keeps the markup valid inside `<p>`, which is
/// where comrak places the inline-hook result.
fn render_indent(i: Indent, writer: &mut dyn Write) -> fmt::Result {
    write!(
        writer,
        r#"<span class="afm-indent afm-indent-{n}" data-amount="{n}"></span>"#,
        n = i.amount,
    )
}

/// Leaf `地付き` (offset 0) / `地からN字上げ` (offset N). Same shape as
/// [`render_indent`]: an empty marker span that the stylesheet turns into
/// a right-aligned block.
fn render_align_end(a: AlignEnd, writer: &mut dyn Write) -> fmt::Result {
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

fn fallback(node: &AozoraNode, writer: &mut dyn Write) -> fmt::Result {
    write!(writer, "<!-- {} -->", node.xml_node_name())
}

/// Minimal HTML5 text escape for the five structural characters.
fn escape_text(text: &str, writer: &mut dyn Write) -> fmt::Result {
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
        AlignEnd, Annotation, AnnotationKind, Bouten, BoutenKind, BoutenPosition, Indent, Ruby,
        TateChuYoko,
    };

    fn render_to_string(node: &AozoraNode) -> String {
        let mut out = String::new();
        render(node, true, &mut out).expect("fmt::Write into String never fails");
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
    fn ruby_reading_with_embedded_gaiji_renders_segmented() {
        use afm_syntax::{Content, Gaiji, Segment};
        let reading = Content::from_segments(vec![
            Segment::Text("く".into()),
            Segment::Gaiji(Gaiji {
                description: "二の字点".into(),
                ucs: Some('〻'),
                mencode: Some("1-2-22".into()),
            }),
        ]);
        let n = AozoraNode::Ruby(Ruby {
            base: "縊".into(),
            reading,
            delim_explicit: false,
        });
        let out = render_to_string(&n);
        // No bare ［＃ should leak; gaiji should be wrapped in afm-gaiji
        assert!(!out.contains("［＃"));
        assert!(out.contains(r#"<span class="afm-gaiji">〻</span>"#));
        assert!(out.contains("<rt>く<span"));
    }

    #[test]
    fn ruby_base_with_kun_yomi_via_annotation_segment_stays_in_content() {
        use crate::test_support::strip_annotation_wrappers;
        use afm_syntax::{Annotation, AnnotationKind, Content, Segment};
        // Classical kun-yomi mark embedded between kanji characters —
        // handled as an Annotation segment here (the dedicated Kaeriten
        // variant is an independent node, not a segment kind per B1).
        let ruby_base = Content::from_segments(vec![
            Segment::Text("言".into()),
            Segment::Annotation(Annotation {
                raw: "［＃二］".into(),
                kind: AnnotationKind::Unknown,
            }),
            Segment::Text("向和".into()),
        ]);
        let n = AozoraNode::Ruby(Ruby {
            base: ruby_base,
            reading: "コトムケヤハス".into(),
            delim_explicit: false,
        });
        let out = render_to_string(&n);
        // Annotation segment wraps in hidden span, so stripping wrappers
        // leaves no bare ［＃ marker.
        let stripped = strip_annotation_wrappers(&out);
        assert!(!stripped.contains("［＃"));
        assert!(out.contains("afm-annotation"));
    }

    #[test]
    fn kaeriten_renders_as_superscript_afm_kaeriten() {
        use afm_syntax::Kaeriten;
        let n = AozoraNode::Kaeriten(Kaeriten { mark: "レ".into() });
        assert_eq!(
            render_to_string(&n),
            r#"<sup class="afm-kaeriten">レ</sup>"#
        );
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
            position: BoutenPosition::Right,
        });
        assert_eq!(
            render_to_string(&n),
            r#"<em class="afm-bouten afm-bouten-goma afm-bouten-right">可哀想</em>"#
        );
    }

    #[test]
    fn bouten_escapes_structural_characters_in_target() {
        let n = AozoraNode::Bouten(Bouten {
            kind: BoutenKind::WavyLine,
            target: "a<b&c".into(),
            position: BoutenPosition::Right,
        });
        assert_eq!(
            render_to_string(&n),
            r#"<em class="afm-bouten afm-bouten-wavy-line afm-bouten-right">a&lt;b&amp;c</em>"#
        );
    }

    #[test]
    fn bouten_left_position_emits_left_modifier() {
        // `［＃「X」の左に傍点］` shape: the marks render on the
        // left-hand side, distinguishable via the afm-bouten-left
        // modifier class so the CSS theme can style each side.
        let n = AozoraNode::Bouten(Bouten {
            kind: BoutenKind::Goma,
            target: "左".into(),
            position: BoutenPosition::Left,
        });
        assert_eq!(
            render_to_string(&n),
            r#"<em class="afm-bouten afm-bouten-goma afm-bouten-left">左</em>"#
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

    // -------------------------------------------------------------
    // Render arms not touched by the integration tests directly.
    // -------------------------------------------------------------

    #[test]
    fn section_break_renders_each_kind_with_stable_slug() {
        use afm_syntax::SectionKind;
        for (kind, slug) in [
            (SectionKind::Choho, "choho"),
            (SectionKind::Dan, "dan"),
            (SectionKind::Spread, "spread"),
        ] {
            let html = render_to_string(&AozoraNode::SectionBreak(kind));
            let expected =
                format!(r#"<div class="afm-section-break afm-section-break-{slug}"></div>"#);
            assert_eq!(html, expected, "kind={kind:?}");
        }
    }

    #[test]
    fn tcy_renders_text_inside_afm_tcy_span() {
        let n = AozoraNode::TateChuYoko(TateChuYoko { text: "25".into() });
        assert_eq!(render_to_string(&n), r#"<span class="afm-tcy">25</span>"#);
    }

    #[test]
    fn tcy_escapes_structural_characters_in_text() {
        let n = AozoraNode::TateChuYoko(TateChuYoko { text: "<&>".into() });
        assert_eq!(
            render_to_string(&n),
            r#"<span class="afm-tcy">&lt;&amp;&gt;</span>"#
        );
    }

    #[test]
    fn gaiji_with_resolved_ucs_emits_single_char() {
        use afm_syntax::Gaiji;
        let n = AozoraNode::Gaiji(Gaiji {
            description: "placeholder".into(),
            ucs: Some('榁'),
            mencode: Some("第3水準1-85-54".into()),
        });
        assert_eq!(render_to_string(&n), r#"<span class="afm-gaiji">榁</span>"#);
    }

    #[test]
    fn gaiji_without_ucs_falls_back_to_description_escaped() {
        use afm_syntax::Gaiji;
        let n = AozoraNode::Gaiji(Gaiji {
            description: "a<b>".into(),
            ucs: None,
            mencode: None,
        });
        assert_eq!(
            render_to_string(&n),
            r#"<span class="afm-gaiji">a&lt;b&gt;</span>"#
        );
    }

    #[test]
    fn double_ruby_plain_content_wraps_academic_brackets() {
        use afm_syntax::DoubleRuby;
        let n = AozoraNode::DoubleRuby(DoubleRuby {
            content: "emphasis".into(),
        });
        assert_eq!(
            render_to_string(&n),
            r#"<span class="afm-double-ruby">≪emphasis≫</span>"#
        );
    }

    #[test]
    fn double_ruby_escapes_structural_characters() {
        use afm_syntax::DoubleRuby;
        let n = AozoraNode::DoubleRuby(DoubleRuby {
            content: "a<b&c".into(),
        });
        assert_eq!(
            render_to_string(&n),
            r#"<span class="afm-double-ruby">≪a&lt;b&amp;c≫</span>"#
        );
    }

    #[test]
    fn container_variants_emit_distinct_class_tokens_on_enter() {
        // render() dispatches to render_container which emits the
        // opening tag on enter and `</div>` on exit. The comrak
        // walker normally drives enter→children→exit; the unit
        // test covers just the two states to pin the class contract.
        use afm_syntax::{Container, ContainerKind};
        let indent = AozoraNode::Container(Container {
            kind: ContainerKind::Indent { amount: 2 },
        });
        let mut open = String::new();
        render(&indent, true, &mut open).unwrap();
        let mut close = String::new();
        render(&indent, false, &mut close).unwrap();
        assert!(
            open.contains("afm-container-indent afm-container-indent-2"),
            "indent open: {open:?}"
        );
        assert!(open.contains(r#"data-amount="2""#), "indent open: {open:?}");
        assert_eq!(close, "</div>");

        // AlignEnd with non-zero offset.
        let align = AozoraNode::Container(Container {
            kind: ContainerKind::AlignEnd { offset: 3 },
        });
        let mut open = String::new();
        render(&align, true, &mut open).unwrap();
        assert!(
            open.contains("afm-container-align-end") && open.contains(r#"data-offset="3""#),
            "align-end open: {open:?}"
        );

        // Keigakomi / Warichu — class-only, no data attributes.
        for (kind, slug) in [
            (ContainerKind::Keigakomi, "afm-container-keigakomi"),
            (ContainerKind::Warichu, "afm-container-warichu"),
        ] {
            let node = AozoraNode::Container(Container { kind });
            let mut open = String::new();
            render(&node, true, &mut open).unwrap();
            assert!(open.contains(slug), "{slug} open: {open:?}");
        }
    }

    #[test]
    fn inline_nodes_skip_emission_on_the_exit_pass() {
        // Non-container nodes must emit nothing on the exit pass —
        // `entering == false` short-circuits. Comrak's tree walker
        // still calls render() for exit events; any extra bytes
        // would corrupt the containing block.
        let n = AozoraNode::PageBreak;
        let mut buf = String::new();
        render(&n, false, &mut buf).unwrap();
        assert!(
            buf.is_empty(),
            "PageBreak must emit nothing on exit, got {buf:?}"
        );

        let ruby = AozoraNode::Ruby(Ruby {
            base: "x".into(),
            reading: "y".into(),
            delim_explicit: false,
        });
        let mut buf = String::new();
        render(&ruby, false, &mut buf).unwrap();
        assert!(
            buf.is_empty(),
            "Ruby must emit nothing on exit, got {buf:?}"
        );
    }

    #[test]
    fn render_to_string_helper_uses_enter_only_pass() {
        // The test-only `render_to_string` wraps `render(node, true,
        // &mut out)` to keep the unit tests terse. Pin the helper
        // here so a future change to the signature is caught
        // explicitly.
        let n = AozoraNode::PageBreak;
        assert_eq!(
            render_to_string(&n),
            r#"<div class="afm-page-break"></div>"#
        );
    }

    #[test]
    fn bouten_kind_slugs_are_stable_across_variants() {
        // Brittle on purpose — if a BoutenKind variant is renamed, the CSS
        // class contract breaks here, before reaching the stylesheet tests.
        for (kind, want_slug) in [
            (BoutenKind::Goma, "goma"),
            (BoutenKind::WhiteSesame, "white-sesame"),
            (BoutenKind::Circle, "circle"),
            (BoutenKind::WhiteCircle, "white-circle"),
            (BoutenKind::DoubleCircle, "double-circle"),
            (BoutenKind::Janome, "janome"),
            (BoutenKind::Cross, "cross"),
            (BoutenKind::WhiteTriangle, "white-triangle"),
            (BoutenKind::WavyLine, "wavy-line"),
            (BoutenKind::UnderLine, "under-line"),
            (BoutenKind::DoubleUnderLine, "double-under-line"),
        ] {
            let html = render_to_string(&AozoraNode::Bouten(Bouten {
                kind,
                target: "x".into(),
                position: BoutenPosition::Right,
            }));
            let expected =
                format!(r#"<em class="afm-bouten afm-bouten-{want_slug} afm-bouten-right">x</em>"#);
            assert_eq!(html, expected, "kind={kind:?}");
        }
    }
}
