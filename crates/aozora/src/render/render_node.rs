#![expect(clippy::expect_used, reason = "fmt::Write into String is infallible")]

//! HTML rendering for individual AST nodes.
//!
//! Emits per-node HTML by reading an owned [`Node`] and resolving every
//! [`StrId`](crate::syntax::ast::StrId) /
//! [`ContentRange`] /
//! [`SegRange`](crate::syntax::ast::SegRange) against a [`NodeStore`].
//!
//! Only the AST-payload-reading emitters live here. The five pure-scalar leaf
//! variants ([`Node::Line`] / [`Node::PageBreak`] /
//! [`Node::BodyEnd`] / [`Node::ForcedBreak`] /
//! [`Node::SectionBreak`]) emit their fixed markup directly (e.g. the
//! section-break slug table). The heading tag writers, the illustration
//! dimension parser, the line renderer, and the text escaper are reused from
//! [`crate::render::spelling::html`].

use core::fmt::{self, Write};

use crate::spec::roman_slug;
use crate::syntax::GaijiCanonical;
use crate::syntax::accent::{compose_accent, compose_accent_dots};
use crate::syntax::ast::{
    AngleQuote, Content, ContentRange, Directive, ForwardFormat, ForwardPayload, Gaiji,
    GaijiCanonicalOwned, Heading, HeadingHint, Illustration, Kaeriten, MarginNote, Node, NodeStore,
    Ruby, Segment, node_is_content_segment,
};
use crate::syntax::format::ForwardOrigin;
use crate::syntax::{DirectiveKind, EnclosureKind, ForwardAttr, RubySide};

use crate::render::classes;
use crate::render::html::render_inline_source;
use crate::render::spelling::html::{
    escape_text, parse_sashie_dimensions, render_line, write_heading_close, write_heading_open,
};

/// Render a single owned [`Node`] into `writer`.
///
/// Every inline / leaf node emits its markup unconditionally: there is no
/// `entering` flag, since containers are driven through `RenderState` and
/// never reach here. `store` is the resolve authority for the node's interned
/// payloads.
///
/// # Errors
///
/// Propagates formatter write errors.
#[cfg(test)]
pub(crate) fn render<W: Write>(node: Node, store: &NodeStore, out: &mut W) -> fmt::Result {
    render_with_depth(node, store, out, 0)
}

pub(super) fn render_with_depth<W: Write>(
    node: Node,
    store: &NodeStore,
    out: &mut W,
    nested_depth: usize,
) -> fmt::Result {
    match node {
        Node::Ruby(r) => render_ruby(&r, store, out, nested_depth),
        Node::Format(f) => render_format(&f, store, out, nested_depth),
        Node::MarginNote(s) => render_side_note(&s, store, out, nested_depth),
        Node::Gaiji(g) => render_gaiji(&g, store, out),
        // Pure-scalar leaves: render directly through the shared lifetime-free
        // helpers / inline byte spellings (the section-break slug table stays
        // keyed by the canonical keyword).
        Node::Line(lf) => render_line(lf, out),
        Node::PageBreak => out.write_str(r#"<div class="aozora-page-break"></div>"#),
        Node::BodyEnd => out.write_str(r#"<div class="aozora-body-end"></div>"#),
        Node::ForcedBreak => out.write_str("<br />"),
        Node::SectionBreak(k) => {
            let slug = roman_slug(k.keyword()).unwrap_or("other");
            write!(
                out,
                r#"<div class="aozora-section-break aozora-section-break-{slug}"></div>"#,
            )
        }
        Node::Directive(a) => render_annotation(a, store, out),
        Node::Kaeriten(k) => render_kaeriten(k, store, out),
        Node::AngleQuote(d) => render_angle_quote(d, store, out, nested_depth),
        Node::Illustration(s) => render_sashie(&s, store, out, nested_depth),
        Node::Heading(h) => render_aozora_heading(&h, store, out, nested_depth),
        Node::HeadingHint(h) => render_heading_hint(h, store, out),
    }
}

// ----------------------------------------------------------------------
// Content resolve layer — resolve a `ContentRange` / `Content` and emit
// its HTML.
// ----------------------------------------------------------------------

/// Render a [`ContentRange`] run (length 1 by construction) by emitting the
/// HTML of each resolved [`Content`].
fn render_content_range<W: Write>(
    range: ContentRange,
    store: &NodeStore,
    out: &mut W,
    nested_depth: usize,
) -> fmt::Result {
    for c in store.resolve_content_range(range) {
        render_content_one(*c, store, out, nested_depth)?;
    }
    Ok(())
}

/// Render a single [`Content`]. A `Plain` run escapes to text; a
/// `Segments` run walks its segments, escaping text and nesting gaiji +
/// directive markup.
fn render_content_one<W: Write>(
    c: Content,
    store: &NodeStore,
    out: &mut W,
    nested_depth: usize,
) -> fmt::Result {
    match c {
        Content::Plain(id) => escape_text(store.resolve_str(id), out),
        Content::Segments(range) => {
            for seg in store.resolve_seg_range(range) {
                match *seg {
                    Segment::Text(id) => escape_text(store.resolve_str(id), out)?,
                    Segment::Gaiji(g) => render_gaiji(&g, store, out)?,
                    Segment::Directive(a) => render_annotation(a, store, out)?,
                    Segment::Node(node) => {
                        render_content_segment_node(node, store, out, nested_depth)?;
                    }
                }
            }
            Ok(())
        }
    }
}

fn render_content_segment_node<W: Write>(
    node: Node,
    store: &NodeStore,
    out: &mut W,
    nested_depth: usize,
) -> fmt::Result {
    debug_assert!(
        node_is_content_segment(node),
        "Content segments contain only phrasing semantic nodes"
    );
    match node {
        Node::Illustration(illustration) => {
            render_sashie_inline(&illustration, store, out, nested_depth)
        }
        _ => render_with_depth(node, store, out, nested_depth),
    }
}

#[cold]
fn render_nested_source_range<W: Write>(
    range: ContentRange,
    store: &NodeStore,
    out: &mut W,
    nested_depth: usize,
) -> fmt::Result {
    match store.content_range_as_plain(range) {
        Some(source) => render_inline_source(source, out, nested_depth),
        None => render_content_range(range, store, out, nested_depth),
    }
}

fn render_format_target<W: Write>(
    format: &ForwardFormat,
    store: &NodeStore,
    out: &mut W,
    nested_depth: usize,
) -> fmt::Result {
    if matches!(format.payload, ForwardPayload::NestedSource) {
        render_nested_source_range(format.target, store, out, nested_depth)
    } else {
        render_content_range(format.target, store, out, nested_depth)
    }
}

fn contains_nested_markup(text: &str) -> bool {
    text.split_once('《')
        .is_some_and(|(_, tail)| tail.contains('》'))
        || text
            .split_once("［＃")
            .is_some_and(|(_, tail)| tail.contains('］'))
        || text
            .split_once('≪')
            .is_some_and(|(_, tail)| tail.contains('≫'))
}

fn render_nested_content_one<W: Write>(
    content: Content,
    store: &NodeStore,
    out: &mut W,
    nested_depth: usize,
) -> fmt::Result {
    match content {
        Content::Plain(id) => {
            let text = store.resolve_str(id);
            if contains_nested_markup(text) {
                render_inline_source(text, out, nested_depth)
            } else {
                escape_text(text, out)
            }
        }
        _ => render_content_one(content, store, out, nested_depth),
    }
}

// ----------------------------------------------------------------------
// Per-variant AST emitters.
// ----------------------------------------------------------------------

/// Render a ruby node to a `<ruby>` element (a left-side ruby classes its
/// `<rt>` for below-the-line placement).
///
/// When `base_emphasis` is set (#384) — a declined forward directive
/// `［＃「X」に傍点/罫囲み/…］` named this ruby's base as its unique referent — the
/// base is wrapped in that attribute's emphasis element **inside** the `<ruby>`,
/// before the `<rt>`, so the emphasis marks the base glyphs and not the reading.
/// The wrapper is derived by reusing [`render_format`] over a synthetic
/// [`ForwardOrigin::SelfContained`] leaf on the base, so every attribute kind
/// (傍点 → `<em>`, 罫囲み → framed `<span>`, 行右小書き / 太字 / 二重傍線 / …) wraps
/// identically; the separate `Referenced` directive leaf still renders nothing,
/// so exactly one styled copy exists (no #228 double-render).
fn render_ruby<W: Write>(
    r: &Ruby,
    store: &NodeStore,
    out: &mut W,
    nested_depth: usize,
) -> fmt::Result {
    out.write_str("<ruby>")?;
    match r.base_emphasis {
        Some(attr) => {
            let deco = ForwardFormat {
                attr,
                target: r.base,
                origin: ForwardOrigin::SelfContained,
                payload: ForwardPayload::None,
            };
            render_format(&deco, store, out, nested_depth)?;
        }
        None => render_content_range(r.base, store, out, nested_depth)?,
    }
    // A left-side ruby (saidoku building block) marks its `<rt>` with a class
    // so a stylesheet can place the reading below; the right-side form is
    // unchanged.
    out.write_str(match r.side {
        RubySide::Left => r#"<rp>(</rp><rt class="aozora-ruby-left">"#,
        RubySide::Right => "<rp>(</rp><rt>",
    })?;
    render_content_range(r.reading, store, out, nested_depth)?;
    out.write_str("</rt><rp>)</rp></ruby>")
}

/// Render a margin note as a `<ruby>` whose `<rt class="aozora-margin-note">`
/// carries the note text. The note's `kind` is ignored in HTML (serialize-only).
fn render_side_note<W: Write>(
    s: &MarginNote,
    store: &NodeStore,
    out: &mut W,
    nested_depth: usize,
) -> fmt::Result {
    out.write_str("<ruby>")?;
    render_content_range(s.base, store, out, nested_depth)?;
    out.write_str(r#"<rp>(</rp><rt class="aozora-margin-note">"#)?;
    render_content_range(s.note, store, out, nested_depth)?;
    out.write_str("</rt><rp>)</rp></ruby>")
}

/// Render a forward-reference emphasis (bouten / combine-upright / font-size /
/// italic / span / bold) to its HTML element.
///
/// A `Referenced` origin emits **nothing** — its
/// target literal already lives in the upstream plain run (or a ruby base), so
/// re-rendering it here would double the text (#228). This is load-bearing and
/// pinned by the curated `Referenced` inputs. A `Detached` decoration (#333) is
/// *not* `Referenced`, so it falls through the gate and renders styled — it is
/// the styled-literal half of a non-adjacent split, and its literal was removed
/// from the plain run, so rendering it here is the sole (correct) copy.
fn render_format<W: Write>(
    f: &ForwardFormat,
    store: &NodeStore,
    out: &mut W,
    nested_depth: usize,
) -> fmt::Result {
    if matches!(f.origin, ForwardOrigin::Referenced) {
        return Ok(());
    }
    match f.attr {
        ForwardAttr::Bouten { kind, position } => {
            out.write_str(r#"<em class="aozora-bouten aozora-bouten-"#)?;
            out.write_str(classes::bouten_kind_slug(kind))?;
            out.write_str(" aozora-bouten-")?;
            out.write_str(classes::bouten_position_slug(position))?;
            out.write_str(r#"">"#)?;
            render_format_target(f, store, out, nested_depth)?;
            out.write_str("</em>")
        }
        ForwardAttr::CombineUpright => {
            out.write_str(r#"<span class="aozora-combine-upright">"#)?;
            render_format_target(f, store, out, nested_depth)?;
            out.write_str("</span>")
        }
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
            render_format_target(f, store, out, nested_depth)?;
            out.write_str("</span>")
        }
        ForwardAttr::Fraction => render_fraction(f, store, out, nested_depth),
        // Enclosures drawn as a CSS-styled span — the glyph / keyword names the
        // kind (serialize-only) and the stylesheet draws the frame, one class per
        // kind: 「□」囲み / ○付き文字 / 点線丸囲み / 二重罫囲み. 罫囲み has no
        // dedicated span class (`None`) and keeps the slug-keyed
        // `aozora-keigakomi-inline` via the semantic fall-through.
        ForwardAttr::Framed(kind) => match framed_span_class(kind) {
            Some(class) => {
                write!(out, r#"<span class="{class}">"#)?;
                render_format_target(f, store, out, nested_depth)?;
                out.write_str("</span>")
            }
            None => render_forward_semantic(f, store, out, nested_depth),
        },
        // ドット付き (#331): compose the addressed letters of the reclaimed run
        // into their precomposed dotted glyphs (ṁ / ṣ) — see `render_accent_dot`.
        ForwardAttr::AccentDot => render_accent_dot(f, store, out, nested_depth),
        // アクサン / ウムラウト: compose the single target letter with its accent
        // mark into the precomposed glyph (é / ö) — see `render_accent`. Its own
        // arm keeps it off the bold catch-all below (the #376/#385 bug class).
        ForwardAttr::Accent(_) => render_accent(f, store, out, nested_depth),
        // 文末より N字上げ揃え: end-align the run. Reuses the line-form's
        // `aozora-align-end` class / `data-offset` so the two scopes style
        // identically; without this explicit arm the run would fall through to
        // the bold default below.
        ForwardAttr::AlignEnd { offset } => {
            write!(
                out,
                r#"<span class="aozora-align-end" data-offset="{offset}">"#
            )?;
            render_format_target(f, store, out, nested_depth)?;
            out.write_str("</span>")
        }
        ForwardAttr::Bold
        | ForwardAttr::Gothic
        | ForwardAttr::Italic
        | ForwardAttr::SuperScript
        | ForwardAttr::SubScript
        | ForwardAttr::SmallScript(_)
        | ForwardAttr::Horizontal
        | ForwardAttr::Caption
        | ForwardAttr::FontSizeAbsolute(_) => render_forward_semantic(f, store, out, nested_depth),
    }
}

fn render_fraction<W: Write>(
    f: &ForwardFormat,
    store: &NodeStore,
    out: &mut W,
    nested_depth: usize,
) -> fmt::Result {
    let slug = roman_slug("分数").unwrap_or("bunsu");
    write!(out, r#"<span class="aozora-{slug}">"#)?;
    if matches!(f.payload, ForwardPayload::NestedSource) {
        render_format_target(f, store, out, nested_depth)?;
    } else {
        match store.content_range_as_plain(f.target) {
            Some(text) => match text.split_once(['/', '／']) {
                Some((numerator, denominator)) => {
                    out.write_str("<sup>")?;
                    escape_text(numerator, out)?;
                    out.write_str("</sup>⁄<sub>")?;
                    escape_text(denominator, out)?;
                    out.write_str("</sub>")?;
                }
                None => escape_text(text, out)?,
            },
            None => render_format_target(f, store, out, nested_depth)?,
        }
    }
    out.write_str("</span>")
}

/// The dedicated `aozora-*` span class for an enclosure that the stylesheet
/// draws around its target, or `None` for [`EnclosureKind::Rule`], which keeps
/// the slug-keyed `aozora-keigakomi-inline` semantic rendering. Exhaustive so a
/// future enclosure kind is compiler-flagged here rather than silently sharing
/// the ruled-frame class.
const fn framed_span_class(kind: EnclosureKind) -> Option<&'static str> {
    match kind {
        EnclosureKind::Rule => None,
        EnclosureKind::Box => Some("aozora-keigakomi-box"),
        EnclosureKind::Circle => Some("aozora-enclosure-circle"),
        EnclosureKind::CircleDotted => Some("aozora-enclosure-circle-dotted"),
        EnclosureKind::DoubleRule => Some("aozora-enclosure-double-rule"),
    }
}

/// Render a forward attribute that maps to a plain semantic element keyed by
/// its canonical-keyword slug (太字 / 斜体 / 上下付き / 小書き / 絶対サイズ / …).
/// The parameterized and bespoke attributes (bouten / font-size / fraction /
/// box / accent-dot / align-end) are handled by their own arms in
/// [`render_format`]; this is the catch-all for the simple styled runs.
fn render_forward_semantic<W: Write>(
    f: &ForwardFormat,
    store: &NodeStore,
    out: &mut W,
    nested_depth: usize,
) -> fmt::Result {
    let attr = f.attr;
    let (el, close) = match attr {
        ForwardAttr::Italic => ("i", "</i>"),
        ForwardAttr::SuperScript => ("sup", "</sup>"),
        ForwardAttr::SubScript => ("sub", "</sub>"),
        ForwardAttr::SmallScript(_)
        | ForwardAttr::Framed(_)
        | ForwardAttr::Gothic
        | ForwardAttr::Horizontal
        | ForwardAttr::Caption
        | ForwardAttr::FontSizeAbsolute(_) => ("span", "</span>"),
        ForwardAttr::Bold => ("b", "</b>"),
        ForwardAttr::Bouten { .. }
        | ForwardAttr::CombineUpright
        | ForwardAttr::FontSize(_)
        | ForwardAttr::Fraction
        | ForwardAttr::AccentDot
        | ForwardAttr::Accent(_)
        | ForwardAttr::AlignEnd { .. } => {
            unreachable!("bespoke forward attributes are rendered before semantic dispatch")
        }
    };
    let slug = roman_slug(attr.keyword()).unwrap_or("futoji");
    write!(out, r#"<{el} class="aozora-{slug}">"#)?;
    render_format_target(f, store, out, nested_depth)?;
    out.write_str(close)
}

/// Render a #331 dotted-letter forward: compose the addressed letters of the
/// reclaimed run into their precomposed glyphs inside an `aozora-accent-dot`
/// span. The selector grammar lives in [`ForwardPayload::AccentBody`]; the shared
/// composer (also the classifier's validator) produces the visible run. A
/// literal class (not slug-derived) keeps this off the `slugs.rs` / Hepburn
/// path; a body-less or structured target falls back to the run verbatim.
fn render_accent_dot<W: Write>(
    f: &ForwardFormat,
    store: &NodeStore,
    out: &mut W,
    nested_depth: usize,
) -> fmt::Result {
    out.write_str(r#"<span class="aozora-accent-dot">"#)?;
    match (store.content_range_as_plain(f.target), f.payload) {
        (Some(run), ForwardPayload::AccentBody(body_id)) => {
            match compose_accent_dots(run, store.resolve_str(body_id)) {
                Some(composed) => escape_text(&composed, out)?,
                // Unreachable post-classify; render the run rather than drop it.
                None => escape_text(run, out)?,
            }
        }
        // A structured / body-less target can't be composed; emit as-is.
        _ => render_format_target(f, store, out, nested_depth)?,
    }
    out.write_str("</span>")
}

/// Render a forward accent-mark forward: compose the single target letter with
/// its accent `mark` into the precomposed glyph (é / ö) inside an
/// `aozora-accent` span. The shared composer (also the classifier's validator)
/// is the single authority; a structured or non-composable target — unreachable
/// post-classify — falls back to the target verbatim rather than dropping it.
fn render_accent<W: Write>(
    f: &ForwardFormat,
    store: &NodeStore,
    out: &mut W,
    nested_depth: usize,
) -> fmt::Result {
    let ForwardAttr::Accent(mark) = f.attr else {
        unreachable!("render_accent only renders an accent attribute")
    };
    out.write_str(r#"<span class="aozora-accent">"#)?;
    if matches!(f.payload, ForwardPayload::NestedSource) {
        render_format_target(f, store, out, nested_depth)?;
        return out.write_str("</span>");
    }
    match store.content_range_as_plain(f.target) {
        Some(run) => {
            let mut chars = run.chars();
            match (chars.next(), chars.next()) {
                (Some(letter), None) => match compose_accent(letter, mark) {
                    Some(glyph) => out.write_char(glyph)?,
                    None => escape_text(run, out)?,
                },
                _ => escape_text(run, out)?,
            }
        }
        // A structured target can't be composed; emit as-is.
        None => render_format_target(f, store, out, nested_depth)?,
    }
    out.write_str("</span>")
}

/// Render a gaiji node to a `<span class="aozora-gaiji">`.
///
/// Reconstructs an [`crate::syntax::GaijiCanonical`] (the `Unresolved` tail's
/// [`StrId`](crate::syntax::ast::StrId) resolves against `store`) and calls
/// its `resolve`, reusing the shared JIS-table lookup. A resolved gaiji emits
/// its `data-codepoint` + glyph; an unresolved one emits its escaped `hint` as
/// `data-description` + body. `standalone` is ignored in HTML (serialize-only).
fn render_gaiji<W: Write>(g: &Gaiji, store: &NodeStore, out: &mut W) -> fmt::Result {
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

fn render_annotation<W: Write>(a: Directive, store: &NodeStore, out: &mut W) -> fmt::Result {
    match a.kind {
        DirectiveKind::EditorNote => {
            // ［＃入力者注(N)］ → a visible 注N superscript. `a.raw` is the whole
            // bracketed directive; recover N (the classifier guaranteed the shape).
            let raw = store.resolve_str(a.raw);
            let n = raw
                .strip_prefix("［＃入力者注(")
                .and_then(|r| r.strip_suffix(")］"))
                .unwrap_or(raw);
            out.write_str(r#"<sup class="aozora-editor-note">注"#)?;
            escape_text(n, out)?;
            return out.write_str("</sup>");
        }
        // Ruby-placement editorial notes: a compact visible marker rather than a
        // hidden span (so they do not vanish), but NOT the annotated run `X` —
        // `X` is typically the immediately-preceding text, so re-emitting it
        // would double-render. The raw bracket (with `X`) round-trips on
        // serialize; the reader sees only the marker.
        DirectiveKind::RubyAttached | DirectiveKind::RubyRetarget => {
            return out.write_str(r#"<sup class="aozora-ruby-note">ルビ</sup>"#);
        }
        DirectiveKind::RubyPairOpen => {
            return out.write_str(r#"<sup class="aozora-ruby-note">左ルビ</sup>"#);
        }
        DirectiveKind::RubyPairClose => {
            // ［＃左に「Y」のルビ付き終わり］ → show the left-ruby reading Y. Y is
            // the gloss (not surrounding text), so it does not double-render;
            // recover it from the raw bracket the classifier guaranteed.
            let raw = store.resolve_str(a.raw);
            let y = raw
                .strip_prefix("［＃左に「")
                .and_then(|r| r.strip_suffix("」のルビ付き終わり］"))
                .unwrap_or(raw);
            out.write_str(r#"<sup class="aozora-ruby-note">左ルビ「"#)?;
            escape_text(y, out)?;
            return out.write_str("」</sup>");
        }
        DirectiveKind::MarginNotePairOpen => {
            // ［＃注記付き］ / ［＃左に注記付き］ → a compact marker at the span
            // start; the note text is named on the matching close. The `左に`
            // form sits on the left, so distinguish the marker label.
            let raw = store.resolve_str(a.raw);
            let label = if raw.starts_with("［＃左に") {
                "左注記"
            } else {
                "注記"
            };
            out.write_str(r#"<sup class="aozora-margin-note">"#)?;
            out.write_str(label)?;
            return out.write_str("</sup>");
        }
        DirectiveKind::MarginNotePairClose => {
            // ［＃「Y」の注記付き終わり］ / ［＃左に「Y」の注記付き終わり］ → show the
            // margin-note text Y. Y is the note (not surrounding text), so it
            // does not double-render; recover it from the raw bracket the
            // classifier guaranteed (may hold a nested ［＃…］ gaiji, echoed
            // as literal notation).
            let raw = store.resolve_str(a.raw);
            let left = raw.starts_with("［＃左に「");
            let (label, prefix) = if left {
                ("左注記", "［＃左に「")
            } else {
                ("注記", "［＃「")
            };
            let y = raw
                .strip_prefix(prefix)
                .and_then(|r| r.strip_suffix("」の注記付き終わり］"))
                .unwrap_or(raw);
            out.write_str(r#"<sup class="aozora-margin-note">"#)?;
            out.write_str(label)?;
            out.write_str("「")?;
            escape_text(y, out)?;
            return out.write_str("」</sup>");
        }
        DirectiveKind::NonCanonical
        | DirectiveKind::Editorial
        | DirectiveKind::Sic
        | DirectiveKind::BaseTextVariant
        | DirectiveKind::WarichuOpen
        | DirectiveKind::WarichuClose
        | DirectiveKind::Empty => {}
    }
    out.write_str(r#"<span class="aozora-directive" hidden>"#)?;
    escape_text(store.resolve_str(a.raw), out)?;
    out.write_str("</span>")
}

/// Render a kaeriten mark as `<sup class="aozora-kaeriten">…</sup>`.
fn render_kaeriten<W: Write>(k: Kaeriten, store: &NodeStore, out: &mut W) -> fmt::Result {
    out.write_str(r#"<sup class="aozora-kaeriten">"#)?;
    escape_text(store.resolve_str(k.mark), out)?;
    out.write_str("</sup>")
}

/// Render an angle-quote as `<span class="aozora-angle-quote">《…》</span>`.
fn render_angle_quote<W: Write>(
    d: AngleQuote,
    store: &NodeStore,
    out: &mut W,
    nested_depth: usize,
) -> fmt::Result {
    out.write_str(r#"<span class="aozora-angle-quote">《"#)?;
    render_content_range(d.content, store, out, nested_depth)?;
    out.write_str("》</span>")
}

/// Render an illustration as a `<figure class="aozora-illustration">` with an
/// `<img>` (optional width/height from the dimensions, alt from the
/// description) and an optional `<figcaption>`. The figure `number` is ignored
/// in HTML (serialize-only).
fn render_sashie<W: Write>(
    s: &Illustration,
    store: &NodeStore,
    out: &mut W,
    nested_depth: usize,
) -> fmt::Result {
    out.write_str(r#"<figure class="aozora-illustration">"#)?;
    render_sashie_image(s, store, out)?;
    if let Some(caption) = s.caption {
        out.write_str("<figcaption>")?;
        render_nested_content_one(caption, store, out, nested_depth)?;
        out.write_str("</figcaption>")?;
    }
    out.write_str("</figure>")
}

fn render_sashie_inline<W: Write>(
    s: &Illustration,
    store: &NodeStore,
    out: &mut W,
    nested_depth: usize,
) -> fmt::Result {
    out.write_str(r#"<span class="aozora-illustration">"#)?;
    render_sashie_image(s, store, out)?;
    if let Some(caption) = s.caption {
        out.write_str(r#"<span class="aozora-illustration-caption">"#)?;
        render_nested_content_one(caption, store, out, nested_depth)?;
        out.write_str("</span>")?;
    }
    out.write_str("</span>")
}

fn render_sashie_image<W: Write>(s: &Illustration, store: &NodeStore, out: &mut W) -> fmt::Result {
    out.write_str(r#"<img src=""#)?;
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
    out.write_str(r#"" />"#)
}

/// Render an Aozora heading by wrapping its text with the shared
/// `write_heading_open` / `write_heading_close` writers from
/// [`crate::render::spelling::html`], keeping the `<hN>` / `<div>` spelling single-source.
fn render_aozora_heading<W: Write>(
    h: &Heading,
    store: &NodeStore,
    out: &mut W,
    nested_depth: usize,
) -> fmt::Result {
    write_heading_open(h.kind, h.style, out)?;
    render_content_range(h.text, store, out, nested_depth)?;
    write_heading_close(h.kind, h.style, out)
}

/// Render a heading hint (`［＃「X」は中見出し］`).
///
/// A referent-present hint that the lowering pass did not promote stays a
/// hidden marker carrying its level / style / target as data attributes. A
/// `self_contained` hint (a no-referent forward heading) instead renders its
/// quoted target visibly, classed as a heading by level — the inline analogue
/// of a promoted `<hN>`, valid where a block heading is not (the directive sits
/// mid-line). Both serialize bracket-only, so the round-trip stays a fixed
/// point.
fn render_heading_hint<W: Write>(h: HeadingHint, store: &NodeStore, out: &mut W) -> fmt::Result {
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
    if h.self_contained {
        // Visible: the quoted run is itself the heading text.
        out.write_str(">")?;
        escape_text(store.resolve_str(h.target), out)?;
        return out.write_str("</span>");
    }
    // Hidden marker: the heading text lives in the (promotable) referent run.
    out.write_str(r#" data-target=""#)?;
    escape_text(store.resolve_str(h.target), out)?;
    out.write_str(r#"" hidden></span>"#)
}

#[cfg(test)]
mod tests {
    //! Byte-exact per-node render pins. Each test renders one owned node (or
    //! calls a private emitter directly) and asserts the exact HTML, hitting
    //! both sides of every node-kind / element decision so a deleted match arm
    //! or stubbed emitter is observable.

    use super::*;
    use crate::render::MAX_NESTED_SOURCE_DEPTH;
    use crate::syntax::alloc::Allocator;
    use crate::syntax::{AccentMark, HeadingKind, HeadingStyle, MarginNoteKind};

    /// Render a full owned node into a fresh `String` (`render` is infallible
    /// over a `String` sink).
    fn html(node: Node, store: &NodeStore) -> String {
        let mut s = String::new();
        render(node, store, &mut s).expect("render into String is infallible");
        s
    }

    /// The pure-scalar `ForcedBreak` leaf emits `<br />`, not the
    /// `<!-- name -->` fallback of an unhandled variant.
    #[test]
    fn forced_break_renders_br() {
        let store = Allocator::new().into_store();
        assert_eq!(html(Node::ForcedBreak, &store), "<br />");
    }

    /// A `Content::Segments` run walks its segments in order, emitting text,
    /// gaiji, and nested-directive markup — each segment arm is load-bearing,
    /// and the whole `Segments` arm is too (its deletion drops the run).
    #[test]
    fn content_segments_renders_each_segment_kind() {
        let mut a = Allocator::new();
        let text = a.seg_text("あ");
        let g = a.make_gaiji("架空", None, false);
        let gaiji = a.seg_gaiji(g);
        let dir = a.make_directive("［＃ママ］", DirectiveKind::Sic);
        let annot = a.seg_annotation(dir);
        let content = a.content_segments(&[text, gaiji, annot]);
        let store = a.into_store();

        let mut out = String::new();
        render_content_one(content, &store, &mut out, 0).expect("render into String is infallible");
        // Text segment (also proves the `Segments` arm did not collapse away).
        assert!(out.contains("あ"), "text segment missing: {out}");
        // Gaiji segment.
        assert!(
            out.contains(r#"class="aozora-gaiji"#),
            "gaiji segment missing: {out}"
        );
        // Nested-directive segment.
        assert!(
            out.contains(r#"class="aozora-directive"#),
            "directive segment missing: {out}"
        );
        // Order is preserved: text, then gaiji, then directive.
        let t = out.find("あ").expect("text present");
        let gi = out.find("aozora-gaiji").expect("gaiji present");
        let di = out.find("aozora-directive").expect("directive present");
        assert!(t < gi && gi < di, "segment order diverged: {out}");
    }

    #[test]
    fn nested_markup_requires_one_complete_delimiter_pair() {
        assert!(!contains_nested_markup("plain text"));
        assert!(contains_nested_markup("語《ご》"));
        assert!(contains_nested_markup("前［＃ママ］後"));
        assert!(contains_nested_markup("前≪引用≫後"));
        assert!(!contains_nested_markup("語《ご"));
        assert!(!contains_nested_markup("前［＃ママ"));
        assert!(!contains_nested_markup("前≪引用"));
    }

    #[test]
    fn illustration_caption_uses_the_same_nested_source_limit() {
        let mut allocator = Allocator::new();
        let caption = allocator.content_plain("［＃「<&」に傍点］");
        let node = allocator.sashie("figure.png", None, None, Some(caption));
        let store = allocator.into_store();

        let normal = html(node, &store);
        assert!(normal.contains(
            r#"<figcaption><em class="aozora-bouten aozora-bouten-goma aozora-bouten-right">&lt;&amp;</em></figcaption>"#
        ));

        let mut limited = String::new();
        render_with_depth(node, &store, &mut limited, MAX_NESTED_SOURCE_DEPTH)
            .expect("render into String is infallible");
        assert!(limited.contains("<figcaption>［＃「&lt;&amp;」に傍点］</figcaption>"));

        let Node::Illustration(illustration) = node else {
            panic!("sashie creates an illustration")
        };
        let mut inline = String::new();
        render_sashie_inline(&illustration, &store, &mut inline, MAX_NESTED_SOURCE_DEPTH)
            .expect("render into String is infallible");
        assert!(inline.contains(
            "<span class=\"aozora-illustration-caption\">［＃「&lt;&amp;」に傍点］</span>"
        ));
    }

    #[test]
    fn gaiji_codepoint_attribute_separates_multiple_scalars() {
        let mut a = Allocator::new();
        let single = a.make_gaiji("々", None, false);
        let single = a.gaiji(single);
        let multiple = a.make_gaiji("", Some("第3水準1-4-87"), false);
        let multiple = a.gaiji(multiple);
        let store = a.into_store();

        assert_eq!(
            html(single, &store),
            r#"<span class="aozora-gaiji" data-codepoint="U+3005">々</span>"#
        );
        assert_eq!(
            html(multiple, &store),
            "<span class=\"aozora-gaiji\" data-codepoint=\"U+304B U+309A\">か\u{309A}</span>"
        );
    }

    /// A margin note renders a full `<ruby>` with the note carried in an
    /// `aozora-margin-note` `<rt>` — the emitter writes real markup, not the
    /// empty `Ok(())` of a stubbed body.
    #[test]
    fn side_note_renders_ruby_markup() {
        let mut a = Allocator::new();
        let base = a.content_plain("未来");
        let gloss = a.content_plain("みらい");
        let node = a.side_note(MarginNoteKind::Gloss, base, gloss);
        let store = a.into_store();
        assert_eq!(
            html(node, &store),
            r#"<ruby>未来<rp>(</rp><rt class="aozora-margin-note">みらい</rt><rp>)</rp></ruby>"#
        );
    }

    /// The semantic fall-through picks the HTML element per attribute kind:
    /// 斜体 → `<i>`, 上付き → `<sup>`, 下付き → `<sub>`, the span family (横組み /
    /// 小書き / 枠 / …) → `<span>`, and the weight default → `<b>`. Deleting any
    /// arm reroutes its attribute to the `<b>` default, changing the tag.
    #[test]
    fn forward_semantic_selects_element_per_attr() {
        let mut a = Allocator::new();
        let mut cases: Vec<(Node, &'static str)> = Vec::new();
        for (attr, tag) in [
            (ForwardAttr::Italic, "i"),
            (ForwardAttr::SuperScript, "sup"),
            (ForwardAttr::SubScript, "sub"),
            (ForwardAttr::Horizontal, "span"),
            (ForwardAttr::Gothic, "span"),
            (ForwardAttr::Bold, "b"),
        ] {
            let t = a.content_plain("字");
            let node = a.forward_format(attr, t, ForwardOrigin::SelfContained);
            cases.push((node, tag));
        }
        let store = a.into_store();
        for (node, tag) in cases {
            let out = html(node, &store);
            let open = format!("<{tag} ");
            let close = format!("</{tag}>");
            assert!(out.starts_with(&open), "element open for {tag}: {out}");
            assert!(out.ends_with(&close), "element close for {tag}: {out}");
            assert!(out.contains("字"), "target text for {tag}: {out}");
        }
    }

    /// A #331 dotted-letter forward composes its addressed letter into the
    /// precomposed dotted glyph (`Sam` + `mは上ドット付き` → `Saṁ`), not the
    /// verbatim run the fall-through would emit.
    #[test]
    fn accent_dot_composes_dotted_glyph() {
        let mut a = Allocator::new();
        let t = a.content_plain("Sam");
        let node = a.accent_dot(t, "mは上ドット付き", ForwardOrigin::SelfContained);
        let store = a.into_store();
        // ṁ is U+1E41 (LATIN SMALL LETTER M WITH DOT ABOVE).
        let expected = "<span class=\"aozora-accent-dot\">Sa\u{1e41}</span>";
        assert_eq!(html(node, &store), expected);
    }

    #[test]
    fn accent_preserves_a_non_singleton_target() {
        let mut a = Allocator::new();
        let target = a.content_plain("ab");
        let node = a.forward_format(
            ForwardAttr::Accent(AccentMark::Acute),
            target,
            ForwardOrigin::SelfContained,
        );
        let store = a.into_store();

        assert_eq!(
            html(node, &store),
            r#"<span class="aozora-accent">ab</span>"#
        );
    }

    /// An Aozora heading wraps its text in the shared `<hN>` open/close writers
    /// — real markup, not the empty `Ok(())` of a stubbed body.
    #[test]
    fn aozora_heading_renders_hn_markup() {
        let mut a = Allocator::new();
        let text = a.content_plain("第一章");
        let node = a.aozora_heading(HeadingKind::Large, HeadingStyle::Standard, text);
        let store = a.into_store();
        assert_eq!(
            html(node, &store),
            r#"<h1 class="aozora-heading aozora-heading-large">第一章</h1>"#
        );
    }

    #[test]
    fn directive_arms_render_exact_markup() {
        let mut a = Allocator::new();
        let cases: Vec<(Directive, &'static str)> = vec![
            (
                a.make_directive("》", DirectiveKind::WarichuClose),
                r#"<span class="aozora-directive" hidden>》</span>"#,
            ),
            (
                a.make_directive("［＃「振」のルビ「ふ」］", DirectiveKind::RubyAttached),
                r#"<sup class="aozora-ruby-note">ルビ</sup>"#,
            ),
            (
                a.make_directive("［＃「振」を「ふ」に置換］", DirectiveKind::RubyRetarget),
                r#"<sup class="aozora-ruby-note">ルビ</sup>"#,
            ),
            (
                a.make_directive("［＃左に「さ」のルビ付き］", DirectiveKind::RubyPairOpen),
                r#"<sup class="aozora-ruby-note">左ルビ</sup>"#,
            ),
            (
                a.make_directive(
                    "［＃左に「さい」のルビ付き終わり］",
                    DirectiveKind::RubyPairClose,
                ),
                r#"<sup class="aozora-ruby-note">左ルビ「さい」</sup>"#,
            ),
            (
                a.make_directive("［＃注記付き］", DirectiveKind::MarginNotePairOpen),
                r#"<sup class="aozora-margin-note">注記</sup>"#,
            ),
            (
                a.make_directive("［＃左に注記付き］", DirectiveKind::MarginNotePairOpen),
                r#"<sup class="aozora-margin-note">左注記</sup>"#,
            ),
            (
                a.make_directive(
                    "［＃「メモ」の注記付き終わり］",
                    DirectiveKind::MarginNotePairClose,
                ),
                r#"<sup class="aozora-margin-note">注記「メモ」</sup>"#,
            ),
            (
                a.make_directive(
                    "［＃左に「メモ」の注記付き終わり］",
                    DirectiveKind::MarginNotePairClose,
                ),
                r#"<sup class="aozora-margin-note">左注記「メモ」</sup>"#,
            ),
        ];
        let store = a.into_store();
        for (d, expected) in cases {
            assert_eq!(
                html(Node::Directive(d), &store),
                expected,
                "kind {:?}",
                d.kind
            );
        }
    }
}
