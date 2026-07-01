//! The CSS class names the HTML renderer can emit.
//!
//! [`AOZORA_CLASSES`] is the authoritative, public list of every
//! `aozora-*` class [`crate::render_node`] / `crate::html` writes.
//! Downstream consumers (e.g. the sibling `afm` crate) import it instead
//! of hand-mirroring the contract. The `class_list_matches_emitted`
//! test renders every `Node` + `ContainerKind` variant and asserts
//! the emitted class tokens are *exactly* this set, so the published
//! list can never drift from the emit sites.

use aozora_syntax::{BoutenKind, BoutenPosition, HeadingKind, HeadingStyle};

/// Every CSS class token the HTML renderer can emit.
///
/// Open-ended numeric variants are normalised to their stem
/// (`aozora-indent-2` → `aozora-indent`, `aozora-align-end-3` →
/// `aozora-align-end`); the fixed slug families (`aozora-bouten-<kind>`,
/// `aozora-section-break-<kind>`, the emphasis slugs, the
/// `aozora-heading-<level>` / `-<style>` modifiers) are listed in full.
/// Sorted and deduplicated.
///
/// Pinned by the `class_list_matches_emitted` test. A consumer that
/// accepts a numeric variant should match `aozora-indent` / etc. as a
/// stem and allow a trailing `-<n>`.
pub const AOZORA_CLASSES: &[&str] = &[
    "aozora-align-end",
    "aozora-angle-quote",
    "aozora-body-end",
    "aozora-bouten",
    "aozora-bouten-batsu",
    "aozora-bouten-bosen",
    "aozora-bouten-goma",
    "aozora-bouten-hasen",
    "aozora-bouten-janome",
    "aozora-bouten-kurosankaku",
    "aozora-bouten-kusarisen",
    "aozora-bouten-left",
    "aozora-bouten-maru",
    "aozora-bouten-namisen",
    "aozora-bouten-nijubosen",
    "aozora-bouten-nijumaru",
    "aozora-bouten-right",
    "aozora-bouten-shirogoma",
    "aozora-bouten-shiromaru",
    "aozora-bouten-shirosankaku",
    "aozora-bunsu",
    "aozora-caption",
    "aozora-center",
    "aozora-combine-upright",
    "aozora-container",
    "aozora-container-align-end",
    "aozora-container-center",
    "aozora-container-columns",
    "aozora-container-font-larger",
    "aozora-container-font-smaller",
    "aozora-container-futoji",
    "aozora-container-indent",
    "aozora-container-keigakomi",
    "aozora-container-line-kumi",
    "aozora-container-line-width",
    "aozora-container-shatai",
    "aozora-container-table",
    "aozora-container-warichu",
    "aozora-container-wrap-indent",
    "aozora-container-yokogumi",
    "aozora-directive",
    "aozora-editor-note",
    "aozora-font-extra-large",
    "aozora-font-large",
    "aozora-font-larger",
    "aozora-font-medium",
    "aozora-font-small",
    "aozora-font-smaller",
    "aozora-futoji",
    "aozora-gaiji",
    "aozora-heading",
    "aozora-heading-hint",
    "aozora-heading-large",
    "aozora-heading-medium",
    "aozora-heading-same-line",
    "aozora-heading-small",
    "aozora-heading-window",
    "aozora-illustration",
    "aozora-indent",
    "aozora-kaeriten",
    "aozora-keigakomi-inline",
    "aozora-kogaki-left",
    "aozora-kogaki-right",
    "aozora-line-font-extra-large",
    "aozora-line-font-large",
    "aozora-line-font-medium",
    "aozora-line-font-small",
    "aozora-line-futoji",
    "aozora-margin-note",
    "aozora-page-break",
    "aozora-ruby-left",
    "aozora-section-break",
    "aozora-section-break-kaicho",
    "aozora-section-break-kaidan",
    "aozora-section-break-kaimihiraki",
    "aozora-shatai",
    "aozora-shitatsuki",
    "aozora-uwatsuki",
    "aozora-warichu",
    "aozora-yokogumi",
];

// ── Per-enum class-token slugs ────────────────────────────────────────
// Single source of truth for the variable `aozora-*` slugs the renderer
// composes, shared by `crate::render_node` (the emit sites) and the
// `class_list_matches_emitted` derivation below. Fixed, structure-bound
// class literals stay inline in the HTML templates they belong to.

/// Romaji slug for a [`BoutenKind`] — the `aozora-bouten-<slug>` suffix.
///
/// The slug lives in the spec table (`RENDER_SLUGS`), keyed by the
/// canonical 青空文庫 keyword; `BoutenKind` is `#[non_exhaustive]`, so an
/// unknown kind falls back to `other` and rendering stays infallible.
#[must_use]
pub(crate) fn bouten_kind_slug(kind: BoutenKind) -> &'static str {
    aozora_spec::roman_slug(kind.keyword()).unwrap_or("other")
}

/// Side slug for a [`BoutenPosition`] — the `aozora-bouten-<slug>` /
/// `aozora-kogaki-<slug>` suffix.
#[must_use]
pub(crate) const fn bouten_position_slug(pos: BoutenPosition) -> &'static str {
    match pos {
        BoutenPosition::Left => "left",
        _ => "right",
    }
}

/// Outline-level slug for a [`HeadingKind`] — the `aozora-heading-<slug>`
/// suffix.
///
/// Shared by the forward-reference leaf and the paired/block heading
/// container. `HeadingKind` is `#[non_exhaustive]`; an unknown level
/// defaults to the top (`large`).
#[must_use]
pub(crate) const fn heading_level_slug(kind: HeadingKind) -> &'static str {
    match kind {
        HeadingKind::Medium => "medium",
        HeadingKind::Small => "small",
        _ => "large",
    }
}

/// Per-style modifier slug for a [`HeadingStyle`].
///
/// `None` for the standard style, which adds no modifier so a standard
/// heading's markup is unchanged. An unknown (`#[non_exhaustive]`) style
/// is treated as standard.
#[must_use]
pub(crate) const fn heading_style_slug(style: HeadingStyle) -> Option<&'static str> {
    match style {
        HeadingStyle::SameLine => Some("same-line"),
        HeadingStyle::Window => Some("window"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{AOZORA_CLASSES, bouten_kind_slug, bouten_position_slug};
    use crate::render_node::render_container;
    use crate::render_node_owned::render_owned;
    use aozora_syntax::alloc_owned::OwnedAllocator;
    use aozora_syntax::owned::{NodeOwned, NodeStore};
    use aozora_syntax::{
        AbsoluteSize, BOUTEN_KINDS, BlockStyles, BoutenKind, BoutenPosition, ColumnCount,
        Container, DirectiveKind, EnclosureKind, FontShift, ForwardAttr, ForwardOrigin,
        HEADING_KINDS, HEADING_STYLES, HeadingKind, HeadingStyle, IndentBlock, IndentLayout, Kumi,
        LineFormat, LineWidth, MarginNoteKind, RegionFormat, SECTION_KINDS,
    };
    use core::num::{NonZeroI8, NonZeroU8};
    use std::collections::BTreeSet;

    fn fs(steps: i8) -> FontShift {
        FontShift(NonZeroI8::new(steps).expect("non-zero"))
    }
    fn lw(n: u8) -> LineWidth {
        LineWidth(NonZeroU8::new(n).expect("non-zero"))
    }
    fn cc(n: u8) -> ColumnCount {
        ColumnCount(NonZeroU8::new(n).expect("non-zero"))
    }
    fn kumi(lines: u8, width: u8) -> Kumi {
        Kumi {
            lines: NonZeroU8::new(lines).expect("non-zero"),
            width: NonZeroU8::new(width).expect("non-zero"),
        }
    }

    /// E1-1: a no-referent forward ([`ForwardOrigin::SelfContained`]) owns the
    /// only copy of its target, so it renders the styled run — it is **not**
    /// short-circuited like [`ForwardOrigin::Referenced`] (the #228 guard). The
    /// producer is wired in E1-2/E1-3; the render fall-through is pinned here so
    /// the plumbing PR carries its own proof.
    #[test]
    fn self_contained_forward_renders_styled_run() {
        let mut a = OwnedAllocator::new();
        let bold_t = a.content_plain("強");
        let bold = a.forward_format(ForwardAttr::Bold, bold_t, ForwardOrigin::SelfContained);
        let bouten_t = a.content_plain("点");
        let bouten = a.bouten(
            BoutenKind::Goma,
            bouten_t,
            BoutenPosition::Right,
            ForwardOrigin::SelfContained,
        );
        let store = a.into_store();

        let mut bold_html = String::new();
        render_owned(bold, &store, &mut bold_html).expect("render into String is infallible");
        assert_eq!(bold_html, r#"<b class="aozora-futoji">強</b>"#);

        let mut bouten_html = String::new();
        render_owned(bouten, &store, &mut bouten_html).expect("render into String is infallible");
        assert_eq!(
            bouten_html,
            r#"<em class="aozora-bouten aozora-bouten-goma aozora-bouten-right">点</em>"#
        );
    }

    /// Pull the `aozora-*` tokens out of every `class="…"` attribute in
    /// `html`, collapsing a trailing `-<digits>` run to its stem so
    /// open-ended numeric variants (indent / align-end amounts) don't
    /// explode the set.
    fn collect_classes(html: &str, into: &mut BTreeSet<String>) {
        let mut rest = html;
        while let Some(p) = rest.find("class=\"") {
            rest = &rest[p + "class=\"".len()..];
            let end = rest.find('"').unwrap_or(rest.len());
            for c in rest[..end].split_whitespace() {
                if !c.starts_with("aozora-") {
                    continue;
                }
                let stem = match c.rfind('-') {
                    Some(i)
                        if i + 1 < c.len() && c[i + 1..].bytes().all(|b| b.is_ascii_digit()) =>
                    {
                        &c[..i]
                    }
                    _ => c,
                };
                into.insert(stem.to_owned());
            }
            rest = &rest[end..];
        }
    }

    /// Collect a built owned node for later rendering (the allocator's store is
    /// still borrowed mutably during the build, so the render pass runs after
    /// `into_store`).
    fn render_into(node: NodeOwned, nodes: &mut Vec<NodeOwned>) {
        nodes.push(node);
    }

    /// Render one collected node, routing containers through the lifetime-free
    /// container tag writer and every other node through the owned renderer.
    fn render_collected(node: NodeOwned, store: &NodeStore, set: &mut BTreeSet<String>) {
        let mut s = String::new();
        if let NodeOwned::Container(c) = node {
            render_container(c, true, &mut s).expect("render into String is infallible");
            render_container(c, false, &mut s).expect("render into String is infallible");
        } else {
            render_owned(node, store, &mut s).expect("render into String is infallible");
        }
        collect_classes(&s, set);
    }

    /// Render every `Node` + `ContainerKind` variant and collect the
    /// emitted `aozora-*` class tokens (numeric variants collapsed to their
    /// stem). The authoritative derivation [`AOZORA_CLASSES`] is pinned to;
    /// enum families enumerate through the `*_KINDS` constants so a new
    /// variant flows in automatically.
    #[allow(
        clippy::too_many_lines,
        reason = "one render call per Node + ContainerKind variant; \
                  splitting would scatter the exhaustive enumeration"
    )]
    fn all_emitted_classes() -> BTreeSet<String> {
        let mut a = OwnedAllocator::new();
        let mut nodes: Vec<NodeOwned> = Vec::new();

        // --- leaf nodes ---
        render_into(a.page_break(), &mut nodes);
        render_into(a.body_end(), &mut nodes);
        render_into(a.forced_break(), &mut nodes);
        render_into(a.kaeriten("一"), &mut nodes);
        render_into(a.line(LineFormat::Center { page: true }), &mut nodes);
        render_into(a.line(LineFormat::Center { page: false }), &mut nodes);
        render_into(a.line(LineFormat::Indent { amount: 2 }), &mut nodes);
        render_into(a.line(LineFormat::AlignEnd { offset: 0 }), &mut nodes);
        render_into(a.line(LineFormat::AlignEnd { offset: 2 }), &mut nodes);
        render_into(a.line(LineFormat::Bold), &mut nodes);
        for size in [
            AbsoluteSize::ExtraLarge,
            AbsoluteSize::Large,
            AbsoluteSize::Medium,
            AbsoluteSize::Small,
        ] {
            render_into(
                a.line(LineFormat::FontSizeAbsolute { size, bold: false }),
                &mut nodes,
            );
        }
        // The `、太字` compound adds the line-bold class (already covered above).
        render_into(
            a.line(LineFormat::FontSizeAbsolute {
                size: AbsoluteSize::Large,
                bold: true,
            }),
            &mut nodes,
        );
        render_into(a.sashie("f.png", None, None, None), &mut nodes);
        render_into(a.sashie_general("f.png", "図", None), &mut nodes);
        render_into(
            a.heading_hint(HeadingKind::Large, HeadingStyle::Standard, "x", false),
            &mut nodes,
        );
        render_into(
            a.heading_hint(HeadingKind::Medium, HeadingStyle::Standard, "x", true),
            &mut nodes,
        );

        for &k in SECTION_KINDS {
            render_into(a.section_break(k), &mut nodes);
        }

        let g = a.make_gaiji("X", None, false);
        render_into(a.gaiji(g), &mut nodes);
        for kind in [
            DirectiveKind::WarichuOpen,
            DirectiveKind::WarichuClose,
            DirectiveKind::Unknown,
        ] {
            let p = a.make_directive("［＃注］", kind);
            render_into(a.annotation(p), &mut nodes);
        }
        // EditorNote renders a visible 注N superscript from its raw shape.
        let editor_note = a.make_directive("［＃入力者注(1)］", DirectiveKind::EditorNote);
        render_into(a.annotation(editor_note), &mut nodes);

        // Nodes needing Content. `content_plain` takes `&mut self` but
        // returns a Content borrowing the arena (not the allocator), so
        // the `&mut` borrow ends before each `&self` constructor runs and
        // plain sequential lets compile.
        let ruby_base = a.content_plain("親");
        let ruby_reading = a.content_plain("おや");
        render_into(a.ruby(ruby_base, ruby_reading), &mut nodes);
        let lruby_base = a.content_plain("子");
        let lruby_reading = a.content_plain("こ");
        render_into(a.left_ruby(lruby_base, lruby_reading), &mut nodes);
        let note_base = a.content_plain("孫");
        let note_text = a.content_plain("注");
        render_into(
            a.side_note(MarginNoteKind::Gloss, note_base, note_text),
            &mut nodes,
        );
        let tcy = a.content_plain("囲");
        render_into(a.tate_chu_yoko(tcy, ForwardOrigin::Reclaimed), &mut nodes);
        let angle = a.content_plain("内");
        render_into(a.angle_quote(angle), &mut nodes);
        for &kind in BOUTEN_KINDS {
            for pos in [BoutenPosition::Right, BoutenPosition::Left] {
                let t = a.content_plain("文");
                render_into(a.bouten(kind, t, pos, ForwardOrigin::Reclaimed), &mut nodes);
            }
        }
        // Every forward-emphasis attribute that produces a distinct class.
        for attr in [
            ForwardAttr::Bold,
            ForwardAttr::Italic,
            ForwardAttr::SuperScript,
            ForwardAttr::SubScript,
            ForwardAttr::SmallScript(BoutenPosition::Right),
            ForwardAttr::SmallScript(BoutenPosition::Left),
            ForwardAttr::Framed(EnclosureKind::Rule),
            ForwardAttr::Horizontal,
            ForwardAttr::Caption,
            ForwardAttr::Fraction,
            ForwardAttr::FontSizeAbsolute(AbsoluteSize::ExtraLarge),
            ForwardAttr::FontSizeAbsolute(AbsoluteSize::Large),
            ForwardAttr::FontSizeAbsolute(AbsoluteSize::Medium),
            ForwardAttr::FontSizeAbsolute(AbsoluteSize::Small),
            ForwardAttr::FontSize(fs(1)),
        ] {
            let t = a.content_plain("強");
            render_into(
                a.forward_format(attr, t, ForwardOrigin::Reclaimed),
                &mut nodes,
            );
        }
        // FontSize positive → font-larger above; its negative magnitude
        // (font-smaller) is the only other class the variant produces.
        let smaller = a.content_plain("小");
        render_into(
            a.forward_format(
                ForwardAttr::FontSize(fs(-1)),
                smaller,
                ForwardOrigin::Reclaimed,
            ),
            &mut nodes,
        );
        for &kind in HEADING_KINDS {
            for &style in HEADING_STYLES {
                let t = a.content_plain("見");
                render_into(a.aozora_heading(kind, style, t), &mut nodes);
            }
        }

        // --- containers (open + close) ---
        let mut containers = vec![
            RegionFormat::Indent(IndentBlock {
                amount: 2,
                wrap: None,
                center: false,
                layout: IndentLayout::None,
                styles: BlockStyles::EMPTY,
            }),
            RegionFormat::Indent(IndentBlock {
                amount: 2,
                wrap: Some(4),
                center: true,
                layout: IndentLayout::None,
                styles: BlockStyles::EMPTY,
            }),
            // #78 line-layout compounds — exercise the new line-kumi class
            // (字詰め reuses the standalone line-width class).
            RegionFormat::Indent(IndentBlock {
                amount: 3,
                wrap: None,
                center: false,
                layout: IndentLayout::Kumi(kumi(1, 20)),
                styles: BlockStyles::EMPTY,
            }),
            RegionFormat::Indent(IndentBlock {
                amount: 8,
                wrap: None,
                center: false,
                layout: IndentLayout::LineWidth(lw(18)),
                styles: BlockStyles::EMPTY,
            }),
            // #78 co-applied style stack — exercises the flat decoration
            // classes (futoji / yokogumi / keigakomi / font-smaller) on one
            // indent `<div>`.
            RegionFormat::Indent(IndentBlock {
                amount: 4,
                wrap: None,
                center: false,
                layout: IndentLayout::None,
                styles: BlockStyles {
                    bold: true,
                    horizontal: true,
                    framed: true,
                    font: Some(FontShift(NonZeroI8::new(-1).unwrap())),
                },
            }),
            RegionFormat::Warichu,
            RegionFormat::Framed(EnclosureKind::Rule),
            RegionFormat::AlignEnd { offset: 0 },
            RegionFormat::AlignEnd { offset: 2 },
            RegionFormat::LineWidth(lw(30)),
            RegionFormat::Bold { padded: false },
            RegionFormat::Bold { padded: true },
            RegionFormat::Italic { padded: false },
            RegionFormat::Italic { padded: true },
            RegionFormat::Columns(cc(2)),
            RegionFormat::Table,
            RegionFormat::Horizontal,
            RegionFormat::FontSize(fs(2)),
            RegionFormat::FontSize(fs(-2)),
            RegionFormat::SmallScript(BoutenPosition::Right),
            RegionFormat::SmallScript(BoutenPosition::Left),
            RegionFormat::Caption { padded: false },
            RegionFormat::Caption { padded: true },
            RegionFormat::CombineUpright,
        ];
        for &kind in BOUTEN_KINDS {
            for position in [BoutenPosition::Right, BoutenPosition::Left] {
                containers.push(RegionFormat::Bouten { kind, position });
            }
        }
        for &level in HEADING_KINDS {
            for &style in HEADING_STYLES {
                containers.push(RegionFormat::Heading {
                    level,
                    style,
                    padded: true,
                });
            }
        }
        for kind in containers {
            render_into(a.container(Container { kind }), &mut nodes);
        }

        // The build is done; take the store and render every collected node.
        let store = a.into_store();
        let mut emitted: BTreeSet<String> = BTreeSet::new();
        for node in nodes {
            render_collected(node, &store, &mut emitted);
        }
        emitted
    }

    #[test]
    fn class_list_matches_emitted() {
        let listed: BTreeSet<String> = AOZORA_CLASSES.iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(
            all_emitted_classes(),
            listed,
            "AOZORA_CLASSES is out of sync with the renderer's emitted classes"
        );

        // The published list must stay sorted + deduplicated so a binary
        // search / diff against it is well-defined downstream.
        let mut sorted = AOZORA_CLASSES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.as_slice(),
            AOZORA_CLASSES,
            "AOZORA_CLASSES must be sorted and free of duplicates"
        );
    }

    #[test]
    fn bouten_kind_slug_covers_every_variant() {
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
            assert_eq!(
                bouten_kind_slug(kind),
                slug,
                "bouten_kind_slug mismatch for {kind:?}"
            );
        }
    }

    #[test]
    fn bouten_position_slug_maps_left_and_right() {
        assert_eq!(bouten_position_slug(BoutenPosition::Left), "left");
        assert_eq!(bouten_position_slug(BoutenPosition::Right), "right");
    }
}
