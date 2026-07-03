//! Lifetime-free HTML render helpers.
//!
//! The shared, AST-payload-free emitters the owned renderers
//! (`crate::render_node_owned` / `crate::html`) reuse: the container
//! open/close tag writers, the single-line layout-directive renderer, the
//! heading-tag writers, the illustration dimension parser, and the text
//! escaper. Every function takes only `Copy` scalar payloads
//! (`RegionFormat` / `Container` / `LineFormat` / `HeadingKind` / `&str`), so
//! the byte spelling is single-source across both renderers.

use core::fmt::{self, Write};

use aozora_syntax::{
    BlockStyles, Container, HeadingKind, HeadingStyle, IndentBlock, IndentLayout, LineFormat,
    RegionFormat,
};

use crate::classes;

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
