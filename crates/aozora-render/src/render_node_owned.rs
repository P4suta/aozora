//! HTML rendering for individual owned-AST nodes.
//!
//! Emits per-node HTML by reading an owned [`NodeOwned`] and resolving every
//! [`StrId`](aozora_syntax::owned::StrId) /
//! [`ContentRange`] /
//! [`SegRange`](aozora_syntax::owned::SegRange) against a [`NodeStore`].
//!
//! Only the AST-payload-reading emitters live here. The five pure-scalar leaf
//! variants ([`NodeOwned::Line`] / [`NodeOwned::PageBreak`] /
//! [`NodeOwned::BodyEnd`] / [`NodeOwned::ForcedBreak`] /
//! [`NodeOwned::SectionBreak`]) emit their fixed markup directly (e.g. the
//! section-break slug table). The heading tag writers, the illustration
//! dimension parser, the line renderer, and the text escaper are reused from
//! [`crate::render_node`].

use core::fmt::{self, Write};

use aozora_syntax::GaijiCanonical;
use aozora_syntax::accent::{compose_accent, compose_accent_dots};
use aozora_syntax::format::ForwardOrigin;
use aozora_syntax::owned::{
    AngleQuoteOwned, ContentOwned, ContentRange, DirectiveOwned, ForwardFormatOwned,
    GaijiCanonicalOwned, GaijiOwned, HeadingHintOwned, HeadingOwned, IllustrationOwned,
    KaeritenOwned, MarginNoteOwned, NodeOwned, NodeStore, RubyOwned, SegmentOwned,
};
use aozora_syntax::{AccentMark, DirectiveKind, EnclosureKind, ForwardAttr, RubySide};

use crate::classes;
use crate::render_node::{
    escape_text, parse_sashie_dimensions, render_line, write_heading_close, write_heading_open,
};

/// Render a single owned [`NodeOwned`] into `writer`.
///
/// Every inline / leaf node emits its markup unconditionally: there is no
/// `entering` flag, since containers are driven through `RenderState` and
/// never reach here. `store` is the resolve authority for the node's interned
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
        // additions) — emit a fallback `<!-- name -->` comment so the rendered
        // HTML stays diagnosable; the node's `xml_node_name` supplies the name.
        _ => write!(out, "<!-- {} -->", node.xml_node_name()),
    }
}

// ----------------------------------------------------------------------
// Content resolve layer — resolve a `ContentRange` / `ContentOwned` and emit
// its HTML.
// ----------------------------------------------------------------------

/// Render a [`ContentRange`] run (length 1 by construction) by emitting the
/// HTML of each resolved [`ContentOwned`].
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

/// Render a single [`ContentOwned`]. A `Plain` run escapes to text; a
/// `Segments` run walks its segments, escaping text and nesting gaiji +
/// directive markup.
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
// Per-variant AST emitters.
// ----------------------------------------------------------------------

/// Render a ruby node to a `<ruby>` element (a left-side ruby classes its
/// `<rt>` for below-the-line placement).
///
/// When `base_emphasis` is set (#384) — a declined forward directive
/// `［＃「X」に傍点/罫囲み/…］` named this ruby's base as its unique referent — the
/// base is wrapped in that attribute's emphasis element **inside** the `<ruby>`,
/// before the `<rt>`, so the emphasis marks the base glyphs and not the reading.
/// The wrapper is derived by reusing [`render_format_owned`] over a synthetic
/// [`ForwardOrigin::SelfContained`] leaf on the base, so every attribute kind
/// (傍点 → `<em>`, 罫囲み → framed `<span>`, 行右小書き / 太字 / 二重傍線 / …) wraps
/// identically; the separate `Referenced` directive leaf still renders nothing,
/// so exactly one styled copy exists (no #228 double-render).
fn render_ruby_owned<W: Write>(r: &RubyOwned, store: &NodeStore, out: &mut W) -> fmt::Result {
    out.write_str("<ruby>")?;
    match r.base_emphasis {
        Some(attr) => {
            let deco = ForwardFormatOwned {
                attr,
                target: r.base,
                origin: ForwardOrigin::SelfContained,
                accent_body: None,
            };
            render_format_owned(&deco, store, out)?;
        }
        None => render_content_range_owned(r.base, store, out)?,
    }
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

/// Render a margin note as a `<ruby>` whose `<rt class="aozora-margin-note">`
/// carries the note text. The note's `kind` is ignored in HTML (serialize-only).
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
        // 分数: split the target on a slash — ASCII `/` or fullwidth `／` (the
        // corpus uses both) — into a `<sup>`/`<sub>` fraction joined by the
        // fraction slash U+2044. The target is plain math text, so it
        // materializes via `content_range_as_plain`.
        ForwardAttr::Fraction => {
            let slug = aozora_spec::roman_slug("分数").unwrap_or("bunsu");
            write!(out, r#"<span class="aozora-{slug}">"#)?;
            match store.content_range_as_plain(f.target) {
                Some(t) => match t.split_once(['/', '／']) {
                    Some((num, den)) => {
                        out.write_str("<sup>")?;
                        escape_text(num, out)?;
                        out.write_str("</sup>⁄<sub>")?;
                        escape_text(den, out)?;
                        out.write_str("</sub>")?;
                    }
                    // No slash (not attested) — emit the target verbatim rather
                    // than fabricate a numerator / denominator.
                    None => escape_text(t, out)?,
                },
                // A structured (non-plain) target can't be split; render it
                // as-is so no content is dropped.
                None => render_content_range_owned(f.target, store, out)?,
            }
            out.write_str("</span>")
        }
        // Enclosures drawn as a CSS-styled span — the glyph / keyword names the
        // kind (serialize-only) and the stylesheet draws the frame, one class per
        // kind: 「□」囲み / ○付き文字 / 点線丸囲み / 二重罫囲み. 罫囲み has no
        // dedicated span class (`None`) and keeps the slug-keyed
        // `aozora-keigakomi-inline` via the semantic fall-through.
        ForwardAttr::Framed(kind) => match framed_span_class(kind) {
            Some(class) => {
                write!(out, r#"<span class="{class}">"#)?;
                render_content_range_owned(f.target, store, out)?;
                out.write_str("</span>")
            }
            None => render_forward_semantic(f, f.attr, store, out),
        },
        // ドット付き (#331): compose the addressed letters of the reclaimed run
        // into their precomposed dotted glyphs (ṁ / ṣ) — see `render_accent_dot`.
        ForwardAttr::AccentDot => render_accent_dot(f, store, out),
        // アクサン / ウムラウト: compose the single target letter with its accent
        // mark into the precomposed glyph (é / ö) — see `render_accent`. Its own
        // arm keeps it off the bold catch-all below (the #376/#385 bug class).
        ForwardAttr::Accent(mark) => render_accent(f, mark, store, out),
        // 文末より N字上げ揃え: end-align the run. Reuses the line-form's
        // `aozora-align-end` class / `data-offset` so the two scopes style
        // identically; without this explicit arm the run would fall through to
        // the bold default below.
        ForwardAttr::AlignEnd { offset } => {
            write!(
                out,
                r#"<span class="aozora-align-end" data-offset="{offset}">"#
            )?;
            render_content_range_owned(f.target, store, out)?;
            out.write_str("</span>")
        }
        // The HTML element is semantic; the `aozora-*` slug comes from the
        // spec slug table, keyed by the canonical keyword.
        attr => render_forward_semantic(f, attr, store, out),
    }
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
/// [`render_format_owned`]; this is the catch-all for the simple styled runs.
fn render_forward_semantic<W: Write>(
    f: &ForwardFormatOwned,
    attr: ForwardAttr,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    let (el, close) = match attr {
        ForwardAttr::Italic => ("i", "</i>"),
        ForwardAttr::SuperScript => ("sup", "</sup>"),
        ForwardAttr::SubScript => ("sub", "</sub>"),
        ForwardAttr::SmallScript(_)
        | ForwardAttr::Framed(_)
        | ForwardAttr::Horizontal
        | ForwardAttr::Caption
        | ForwardAttr::FontSizeAbsolute(_) => ("span", "</span>"),
        // Bold and any future weight default to the bold element.
        _ => ("b", "</b>"),
    };
    let slug = aozora_spec::roman_slug(attr.keyword()).unwrap_or("futoji");
    write!(out, r#"<{el} class="aozora-{slug}">"#)?;
    render_content_range_owned(f.target, store, out)?;
    out.write_str(close)
}

/// Render a #331 dotted-letter forward: compose the addressed letters of the
/// reclaimed run into their precomposed glyphs inside an `aozora-accent-dot`
/// span. The selector grammar lives in the interned `accent_body`; the shared
/// composer (also the classifier's validator) produces the visible run. A
/// literal class (not slug-derived) keeps this off the `slugs.rs` / Hepburn
/// path; a body-less or structured target falls back to the run verbatim.
fn render_accent_dot<W: Write>(
    f: &ForwardFormatOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    out.write_str(r#"<span class="aozora-accent-dot">"#)?;
    match (store.content_range_as_plain(f.target), f.accent_body) {
        (Some(run), Some(body_id)) => match compose_accent_dots(run, store.resolve_str(body_id)) {
            Some(composed) => escape_text(&composed, out)?,
            // Unreachable post-classify; render the run rather than drop it.
            None => escape_text(run, out)?,
        },
        // A structured / body-less target can't be composed; emit as-is.
        _ => render_content_range_owned(f.target, store, out)?,
    }
    out.write_str("</span>")
}

/// Render a forward accent-mark forward: compose the single target letter with
/// its accent `mark` into the precomposed glyph (é / ö) inside an
/// `aozora-accent` span. The shared composer (also the classifier's validator)
/// is the single authority; a structured or non-composable target — unreachable
/// post-classify — falls back to the target verbatim rather than dropping it.
fn render_accent<W: Write>(
    f: &ForwardFormatOwned,
    mark: AccentMark,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    out.write_str(r#"<span class="aozora-accent">"#)?;
    match store.content_range_as_plain(f.target) {
        Some(run) => match run.chars().next().and_then(|c| compose_accent(c, mark)) {
            Some(glyph) => out.write_char(glyph)?,
            // Unreachable post-classify (validated single composable letter);
            // emit the run rather than drop it.
            None => escape_text(run, out)?,
        },
        // A structured target can't be composed; emit as-is.
        None => render_content_range_owned(f.target, store, out)?,
    }
    out.write_str("</span>")
}

/// Render a gaiji node to a `<span class="aozora-gaiji">`.
///
/// Reconstructs an [`aozora_syntax::GaijiCanonical`] (the `Unresolved` tail's
/// [`StrId`](aozora_syntax::owned::StrId) resolves against `store`) and calls
/// its `resolve`, reusing the shared JIS-table lookup. A resolved gaiji emits
/// its `data-codepoint` + glyph; an unresolved one emits its escaped `hint` as
/// `data-description` + body. `standalone` is ignored in HTML (serialize-only).
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

/// Render a directive: warichu open/close to `<span class="aozora-warichu">` /
/// `</span>`, an editor note to a visible `注N` superscript, and any other
/// directive to a hidden `<span class="aozora-directive">` of its raw text.
fn render_annotation_owned<W: Write>(
    a: DirectiveOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    match a.kind {
        DirectiveKind::WarichuOpen => return out.write_str(r#"<span class="aozora-warichu">"#),
        DirectiveKind::WarichuClose => return out.write_str("</span>"),
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
        _ => {}
    }
    out.write_str(r#"<span class="aozora-directive" hidden>"#)?;
    escape_text(store.resolve_str(a.raw), out)?;
    out.write_str("</span>")
}

/// Render a kaeriten mark as `<sup class="aozora-kaeriten">…</sup>`.
fn render_kaeriten_owned<W: Write>(
    k: KaeritenOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    out.write_str(r#"<sup class="aozora-kaeriten">"#)?;
    escape_text(store.resolve_str(k.mark), out)?;
    out.write_str("</sup>")
}

/// Render an angle-quote as `<span class="aozora-angle-quote">《…》</span>`.
fn render_angle_quote_owned<W: Write>(
    d: AngleQuoteOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    out.write_str(r#"<span class="aozora-angle-quote">《"#)?;
    render_content_range_owned(d.content, store, out)?;
    out.write_str("》</span>")
}

/// Render an illustration as a `<figure class="aozora-illustration">` with an
/// `<img>` (optional width/height from the dimensions, alt from the
/// description) and an optional `<figcaption>`. The figure `number` is ignored
/// in HTML (serialize-only).
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

/// Render an Aozora heading by wrapping its text with the shared
/// `write_heading_open` / `write_heading_close` writers from
/// [`crate::render_node`], keeping the `<hN>` / `<div>` spelling single-source.
fn render_aozora_heading_owned<W: Write>(
    h: &HeadingOwned,
    store: &NodeStore,
    out: &mut W,
) -> fmt::Result {
    write_heading_open(h.kind, h.style, out)?;
    render_content_range_owned(h.text, store, out)?;
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
