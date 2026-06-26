//! HTML rendering for individual owned-AST nodes.
//!
//! Owned mirror of [`crate::render_node`]'s inline / block-leaf path: the
//! same per-node HTML, but reading an owned [`NodeOwned`] and resolving every
//! [`StrId`](aozora_syntax::owned::StrId) /
//! [`ContentRange`] /
//! [`SegRange`](aozora_syntax::owned::SegRange) against a [`NodeStore`]
//! instead of borrowing `&'src str` / `NodeRef<'src>`.
//!
//! Following the P0.2a serializer split, only the AST-payload-reading
//! emitters fork here. The five pure-scalar leaf variants
//! ([`NodeOwned::Line`] / [`NodeOwned::PageBreak`] / [`NodeOwned::BodyEnd`] /
//! [`NodeOwned::ForcedBreak`] / [`NodeOwned::SectionBreak`]) synthesize a
//! lifetime-free borrowed `Node` and delegate to `render_node::render`,
//! keeping a single byte-spelling authority (e.g. the section-break slug
//! table). The heading tag writers, illustration dimension parser, and text
//! escaper are reused from [`crate::render_node`] verbatim.
//!
//! Proven byte-identical to the borrowed renderer by the differential gate in
//! `crates/aozora/tests/owned_html_gate.rs`.

use core::fmt::{self, Write};

use aozora_syntax::GaijiCanonical;
use aozora_syntax::format::ForwardOrigin;
use aozora_syntax::owned::{
    AngleQuoteOwned, ContentOwned, ContentRange, DirectiveOwned, ForwardFormatOwned,
    GaijiCanonicalOwned, GaijiOwned, HeadingHintOwned, HeadingOwned, IllustrationOwned,
    KaeritenOwned, MarginNoteOwned, NodeOwned, NodeStore, RubyOwned, SegmentOwned,
};
use aozora_syntax::{DirectiveKind, ForwardAttr, RubySide};

use crate::classes;
use crate::render_node::{
    escape_text, parse_sashie_dimensions, render_line, write_heading_close, write_heading_open,
};

/// Render a single owned [`NodeOwned`] into `writer`.
///
/// Owned mirror of `render_node::render`'s inline / block-leaf path: every
/// inline / leaf node emits its markup unconditionally (there is no
/// `entering` flag — containers are driven through `RenderState` and never
/// reach here, so this only ever runs the borrowed renderer's `entering ==
/// true` arms). `store` is the resolve authority for the node's interned
/// payloads.
///
/// # Errors
///
/// Propagates formatter write errors.
pub(crate) fn render_owned<W: Write>(
    node: NodeOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    match node {
        NodeOwned::Ruby(r) => render_ruby_owned(&r, store, out),
        NodeOwned::Format(f) => render_format_owned(&f, store, out),
        NodeOwned::MarginNote(s) => render_side_note_owned(&s, store, out),
        NodeOwned::Gaiji(g) => render_gaiji_owned(&g, store, out),
        // Pure-scalar leaves: render directly through the shared lifetime-free
        // helpers / inline byte spellings (the section-break slug table stays
        // keyed by the canonical keyword).
        NodeOwned::Line(lf) => render_line(lf, out),
        NodeOwned::PageBreak => out.write_str(r#"<div class="aozora-page-break"></div>"#),
        NodeOwned::BodyEnd => out.write_str(r#"<div class="aozora-body-end"></div>"#),
        NodeOwned::ForcedBreak => out.write_str("<br />"),
        NodeOwned::SectionBreak(k) => {
            let slug = aozora_spec::roman_slug(k.keyword()).unwrap_or("other");
            write!(
                out,
                r#"<div class="aozora-section-break aozora-section-break-{slug}"></div>"#,
            )
        }
        NodeOwned::Directive(a) => render_annotation_owned(a, store, out),
        NodeOwned::Kaeriten(k) => render_kaeriten_owned(k, store, out),
        NodeOwned::AngleQuote(d) => render_angle_quote_owned(d, store, out),
        NodeOwned::Illustration(s) => render_sashie_owned(&s, store, out),
        NodeOwned::Heading(h) => render_aozora_heading_owned(&h, store, out),
        NodeOwned::HeadingHint(h) => render_heading_hint_owned(h, store, out),
        // Other variants (`Warichu`, `Container`, future non-exhaustive
        // additions) — emit a fallback comment so the rendered HTML stays
        // diagnosable, mirroring the borrowed renderer's `fallback`.
        // `Warichu` / `Container` carry lifetime payloads that cannot be
        // re-synthesized into a borrowed `Node`, so the comment is written
        // directly; `NodeOwned::xml_node_name` is value-for-value identical to
        // the borrowed name, so the bytes match.
        _ => write!(out, "<!-- {} -->", node.xml_node_name()),
    }
}

// ----------------------------------------------------------------------
// Content resolve layer — owned mirror of `render_node::render_content`.
// ----------------------------------------------------------------------

/// Owned mirror of `render_node::render_content` over a [`ContentRange`] run
/// (a borrowed `NonEmpty<Content>` field — length 1 by construction).
fn render_content_range_owned<W: Write>(
    range: ContentRange,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    for c in store.resolve_content_range(range) {
        render_content_one_owned(*c, store, out)?;
    }
    Ok(())
}

/// Owned mirror of `render_node::render_content` for a single
/// [`ContentOwned`]. A `Plain` run escapes to text; a `Segments` run walks its
/// segments (text escaped, gaiji + directive nested) exactly like the
/// borrowed three-arm match.
fn render_content_one_owned<W: Write>(
    c: ContentOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    match c {
        ContentOwned::Plain(id) => escape_text(store.resolve_str(id), out),
        ContentOwned::Segments(range) => {
            for seg in store.resolve_seg_range(range) {
                match *seg {
                    SegmentOwned::Text(id) => escape_text(store.resolve_str(id), out)?,
                    SegmentOwned::Gaiji(g) => render_gaiji_owned(&g, store, out)?,
                    SegmentOwned::Directive(a) => render_annotation_owned(a, store, out)?,
                    // `SegmentOwned` is `#[non_exhaustive]`; forward-compat skip.
                    _ => {}
                }
            }
            Ok(())
        }
        // `ContentOwned` is `#[non_exhaustive]`; forward-compat skip.
        _ => Ok(()),
    }
}

// ----------------------------------------------------------------------
// Per-variant AST emitters — owned mirrors of the `render_*` family.
// ----------------------------------------------------------------------

/// Owned mirror of `render_node::render_ruby`.
fn render_ruby_owned<W: Write>(r: &RubyOwned, store: &NodeStore, out: &mut W) -> fmt::Result {
    out.write_str("<ruby>")?;
    render_content_range_owned(r.base, store, out)?;
    // A left-side ruby (saidoku building block) marks its `<rt>` with a class
    // so a stylesheet can place the reading below; the right-side form is
    // unchanged.
    out.write_str(match r.side {
        RubySide::Left => r#"<rp>(</rp><rt class="aozora-ruby-left">"#,
        _ => "<rp>(</rp><rt>",
    })?;
    render_content_range_owned(r.reading, store, out)?;
    out.write_str("</rt><rp>)</rp></ruby>")
}

/// Owned mirror of `render_node::render_side_note`. The note's `kind` is
/// ignored in HTML (it is serialize-only), matching the borrowed renderer.
fn render_side_note_owned<W: Write>(
    s: &MarginNoteOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    out.write_str("<ruby>")?;
    render_content_range_owned(s.base, store, out)?;
    out.write_str(r#"<rp>(</rp><rt class="aozora-margin-note">"#)?;
    render_content_range_owned(s.note, store, out)?;
    out.write_str("</rt><rp>)</rp></ruby>")
}

/// Owned mirror of `render_node::render_format` (forward-reference emphasis).
///
/// A `Referenced` origin emits **nothing** — its
/// target literal already lives in the upstream plain run (or a ruby base), so
/// re-rendering it here would double the text (#228). This is load-bearing and
/// pinned by the differential gate's `Referenced` corpus / curated inputs.
fn render_format_owned<W: Write>(
    f: &ForwardFormatOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    if matches!(f.origin, ForwardOrigin::Referenced) {
        return Ok(());
    }
    match f.attr {
        ForwardAttr::Bouten { kind, position } => {
            write!(
                out,
                r#"<em class="aozora-bouten aozora-bouten-{kind} aozora-bouten-{pos}">"#,
                kind = classes::bouten_kind_slug(kind),
                pos = classes::bouten_position_slug(position),
            )?;
            render_content_range_owned(f.target, store, out)?;
            out.write_str("</em>")
        }
        ForwardAttr::CombineUpright => {
            out.write_str(r#"<span class="aozora-combine-upright">"#)?;
            render_content_range_owned(f.target, store, out)?;
            out.write_str("</span>")
        }
        // 文字サイズ carries a magnitude, so its open tag is dynamic.
        ForwardAttr::FontSize(shift) => {
            let class = if shift.larger() {
                "aozora-font-larger"
            } else {
                "aozora-font-smaller"
            };
            write!(
                out,
                r#"<span class="{class}" data-steps="{}">"#,
                shift.magnitude()
            )?;
            render_content_range_owned(f.target, store, out)?;
            out.write_str("</span>")
        }
        // The HTML element is semantic; the `aozora-*` slug comes from the
        // spec slug table, keyed by the canonical keyword.
        attr => {
            let (el, close) = match attr {
                ForwardAttr::Italic => ("i", "</i>"),
                ForwardAttr::SuperScript => ("sup", "</sup>"),
                ForwardAttr::SubScript => ("sub", "</sub>"),
                ForwardAttr::SmallScript(_)
                | ForwardAttr::Framed
                | ForwardAttr::Horizontal
                | ForwardAttr::Caption => ("span", "</span>"),
                // Bold and any future weight default to the bold element.
                _ => ("b", "</b>"),
            };
            let slug = aozora_spec::roman_slug(attr.keyword()).unwrap_or("futoji");
            write!(out, r#"<{el} class="aozora-{slug}">"#)?;
            render_content_range_owned(f.target, store, out)?;
            out.write_str(close)
        }
    }
}

/// Owned mirror of `render_node::render_gaiji`.
///
/// Reuses the borrowed resolution authority by reconstructing a borrowed
/// [`GaijiCanonical`] (the `Unresolved` tail's
/// [`StrId`](aozora_syntax::owned::StrId) resolves against `store`) and calling
/// its `resolve`, so the JIS-table lookup is not forked. `standalone` is
/// ignored in HTML (serialize-only). The resolved / fallback bodies are
/// byte-for-byte the borrowed code.
fn render_gaiji_owned<W: Write>(g: &GaijiOwned, store: &NodeStore, out: &mut W) -> fmt::Result {
    let hint = store.resolve_str(g.hint);
    let canonical = match g.canonical {
        GaijiCanonicalOwned::MenKuTen(m) => GaijiCanonical::MenKuTen(m),
        GaijiCanonicalOwned::Unicode(c) => GaijiCanonical::Unicode(c),
        GaijiCanonicalOwned::Unresolved { mencode } => GaijiCanonical::Unresolved {
            mencode: mencode.map(|id| store.resolve_str(id)),
        },
    };
    if let Some(resolved) = canonical.resolve(hint) {
        out.write_str(r#"<span class="aozora-gaiji" data-codepoint=""#)?;
        // Round-trip Resolved through a tiny String buffer so we can iterate
        // its scalars without re-implementing the Char/Multi enum split.
        let mut buf = String::with_capacity(8);
        resolved
            .write_to(&mut buf)
            .expect("Resolved::write_to into String never fails");
        let mut first = true;
        for c in buf.chars() {
            if !first {
                out.write_char(' ')?;
            }
            first = false;
            write!(out, "U+{:04X}", c as u32)?;
        }
        out.write_str(r#"">"#)?;
        resolved.write_to(out)?;
    } else {
        out.write_str(r#"<span class="aozora-gaiji" data-description=""#)?;
        escape_text(hint, out)?;
        out.write_str(r#"">"#)?;
        escape_text(hint, out)?;
    }
    out.write_str("</span>")
}

/// Owned mirror of `render_node::render_annotation`.
fn render_annotation_owned<W: Write>(
    a: DirectiveOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    match a.kind {
        DirectiveKind::WarichuOpen => return out.write_str(r#"<span class="aozora-warichu">"#),
        DirectiveKind::WarichuClose => return out.write_str("</span>"),
        _ => {}
    }
    out.write_str(r#"<span class="aozora-directive" hidden>"#)?;
    escape_text(store.resolve_str(a.raw), out)?;
    out.write_str("</span>")
}

/// Owned mirror of `render_node::render_kaeriten`.
fn render_kaeriten_owned<W: Write>(
    k: KaeritenOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    out.write_str(r#"<sup class="aozora-kaeriten">"#)?;
    escape_text(store.resolve_str(k.mark), out)?;
    out.write_str("</sup>")
}

/// Owned mirror of `render_node::render_angle_quote`.
fn render_angle_quote_owned<W: Write>(
    d: AngleQuoteOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    out.write_str(r#"<span class="aozora-angle-quote">《"#)?;
    render_content_range_owned(d.content, store, out)?;
    out.write_str("》</span>")
}

/// Owned mirror of `render_node::render_sashie`. The figure `number` is
/// ignored in HTML (serialize-only), matching the borrowed renderer.
fn render_sashie_owned<W: Write>(
    s: &IllustrationOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    out.write_str(r#"<figure class="aozora-illustration"><img src=""#)?;
    escape_text(store.resolve_str(s.file), out)?;
    out.write_char('"')?;
    if let Some((w, h)) = s
        .dimensions
        .map(|id| store.resolve_str(id))
        .and_then(parse_sashie_dimensions)
    {
        write!(out, r#" width="{w}" height="{h}""#)?;
    }
    // The general image form's leading description is the alt; the keyword
    // 挿絵 form carries none, so alt stays empty.
    out.write_str(r#" alt=""#)?;
    if let Some(description) = s.description {
        escape_text(store.resolve_str(description), out)?;
    }
    out.write_str(r#"" />"#)?;
    if let Some(caption) = s.caption {
        out.write_str("<figcaption>")?;
        render_content_one_owned(caption, store, out)?;
        out.write_str("</figcaption>")?;
    }
    out.write_str("</figure>")
}

/// Owned mirror of `render_node::render_aozora_heading`. Reuses the borrowed
/// heading-tag writers so the `<hN>` / `<div>` spelling stays single-source.
fn render_aozora_heading_owned<W: Write>(
    h: &HeadingOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    write_heading_open(h.kind, h.style, out)?;
    render_content_range_owned(h.text, store, out)?;
    write_heading_close(h.kind, h.style, out)
}

/// Owned mirror of `render_node::render_heading_hint`.
fn render_heading_hint_owned<W: Write>(
    h: HeadingHintOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    write!(
        out,
        r#"<span class="aozora-heading-hint" data-level="{level}""#,
        level = h.level.outline_level(),
    )?;
    // `data-style` is emitted only for a non-standard style, so a standard
    // hint's markup is unchanged.
    if let Some(style) = classes::heading_style_slug(h.style) {
        write!(out, r#" data-style="{style}""#)?;
    }
    out.write_str(r#" data-target=""#)?;
    escape_text(store.resolve_str(h.target), out)?;
    out.write_str(r#"" hidden></span>"#)
}
