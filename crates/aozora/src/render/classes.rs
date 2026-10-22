//! Defines renderer class names and the public [`AOZORA_CLASSES`] catalogue.

use crate::spec::roman_slug;
use crate::syntax::{BoutenKind, BoutenPosition, HeadingKind, HeadingStyle};

/// CSS classes that the HTML renderer can emit, sorted without duplicates.
///
/// Numeric variants use a shared stem, such as `aozora-indent` for `aozora-indent-2`.
/// Fixed class names appear in full.
pub const AOZORA_CLASSES: &[&str] = &[
    "aozora-accent",
    "aozora-accent-dot",
    "aozora-align-end",
    "aozora-angle-quote",
    "aozora-body-end",
    "aozora-bouten",
    "aozora-bouten-batsu",
    "aozora-bouten-bosen",
    "aozora-bouten-both",
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
    "aozora-container-goshikku",
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
    "aozora-enclosure-circle",
    "aozora-enclosure-circle-dotted",
    "aozora-enclosure-double-rule",
    "aozora-font-extra-large",
    "aozora-font-large",
    "aozora-font-larger",
    "aozora-font-medium",
    "aozora-font-small",
    "aozora-font-smaller",
    "aozora-futoji",
    "aozora-gaiji",
    "aozora-goshikku",
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
    "aozora-keigakomi-box",
    "aozora-keigakomi-inline",
    "aozora-kogaki-left",
    "aozora-kogaki-right",
    "aozora-line-font-extra-large",
    "aozora-line-font-large",
    "aozora-line-font-medium",
    "aozora-line-font-small",
    "aozora-line-futoji",
    "aozora-line-goshikku",
    "aozora-margin-note",
    "aozora-page-break",
    "aozora-ruby-left",
    "aozora-ruby-note",
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

#[must_use]
pub(crate) fn bouten_kind_slug(kind: BoutenKind) -> &'static str {
    roman_slug(kind.keyword()).unwrap_or("other")
}

#[must_use]
pub(crate) const fn bouten_position_slug(pos: BoutenPosition) -> &'static str {
    match pos {
        BoutenPosition::Right => "right",
        BoutenPosition::Left => "left",
        BoutenPosition::Both => "both",
    }
}

#[must_use]
pub(crate) const fn heading_level_slug(kind: HeadingKind) -> &'static str {
    match kind {
        HeadingKind::Large => "large",
        HeadingKind::Medium => "medium",
        HeadingKind::Small => "small",
    }
}

#[must_use]
pub(crate) const fn heading_style_slug(style: HeadingStyle) -> Option<&'static str> {
    match style {
        HeadingStyle::Standard => None,
        HeadingStyle::SameLine => Some("same-line"),
        HeadingStyle::Window => Some("window"),
    }
}

#[cfg(test)]
mod tests {
    use super::{AOZORA_CLASSES, bouten_kind_slug, bouten_position_slug};
    use crate::render::render_node::render;
    use crate::render::spelling::html::{RenderState, render_container};
    use crate::syntax::alloc::Allocator;
    use crate::syntax::ast::{Node, NodeStore};
    use crate::syntax::{
        AbsoluteSize, AccentMark, BOUTEN_KINDS, BlockStyles, BoutenKind, BoutenPosition,
        ColumnCount, Container, DirectiveKind, EnclosureKind, FontShift, ForwardAttr,
        ForwardOrigin, HEADING_KINDS, HEADING_STYLES, HeadingKind, HeadingStyle, IndentBlock,
        IndentLayout, Kumi, LineFormat, LineWidth, MarginNoteKind, RegionFormat, SECTION_KINDS,
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

    #[test]
    fn self_contained_forward_renders_styled_run() {
        let mut a = Allocator::new();
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
        render(bold, &store, &mut bold_html).expect("render into String is infallible");
        assert_eq!(bold_html, r#"<b class="aozora-futoji">強</b>"#);

        let mut bouten_html = String::new();
        render(bouten, &store, &mut bouten_html).expect("render into String is infallible");
        assert_eq!(
            bouten_html,
            r#"<em class="aozora-bouten aozora-bouten-goma aozora-bouten-right">点</em>"#
        );
    }

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

    fn render_into(node: Node, nodes: &mut Vec<Node>) {
        nodes.push(node);
    }

    fn render_collected(node: Node, store: &NodeStore, set: &mut BTreeSet<String>) {
        let mut s = String::new();
        render(node, store, &mut s).expect("render into String is infallible");
        collect_classes(&s, set);
    }

    fn all_emitted_classes() -> BTreeSet<String> {
        let mut a = Allocator::new();
        let mut nodes = leaf_nodes(&mut a);
        nodes.extend(annotation_nodes(&mut a));
        nodes.extend(content_nodes(&mut a));
        nodes.extend(forward_nodes(&mut a));
        nodes.extend(accent_nodes(&mut a));
        let containers = class_containers();
        let store = a.into_store();
        let mut emitted: BTreeSet<String> = BTreeSet::new();
        for node in nodes {
            render_collected(node, &store, &mut emitted);
        }
        for kind in containers {
            let mut html = String::new();
            let container = Container { kind };
            render_container(container, true, &mut html).expect("render into String is infallible");
            render_container(container, false, &mut html)
                .expect("render into String is infallible");
            collect_classes(&html, &mut emitted);
        }
        let mut html = String::new();
        let mut state = RenderState::default();
        state
            .open_warichu(&mut html)
            .expect("render into String is infallible");
        state
            .close_warichu(&mut html)
            .expect("render into String is infallible");
        collect_classes(&html, &mut emitted);
        emitted
    }

    fn leaf_nodes(a: &mut Allocator) -> Vec<Node> {
        let mut nodes = Vec::new();
        render_into(a.page_break(), &mut nodes);
        render_into(a.body_end(), &mut nodes);
        render_into(a.forced_break(), &mut nodes);
        render_into(a.kaeriten("一"), &mut nodes);
        render_into(a.line(LineFormat::Center { page: true }), &mut nodes);
        render_into(a.line(LineFormat::Center { page: false }), &mut nodes);
        render_into(
            a.line(LineFormat::Indent {
                amount: 2,
                end_offset: None,
            }),
            &mut nodes,
        );
        render_into(
            a.line(LineFormat::Indent {
                amount: 2,
                end_offset: Some(2),
            }),
            &mut nodes,
        );
        render_into(a.line(LineFormat::AlignEnd { offset: 0 }), &mut nodes);
        render_into(a.line(LineFormat::AlignEnd { offset: 2 }), &mut nodes);
        render_into(a.line(LineFormat::Gothic), &mut nodes);
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

        nodes
    }

    fn annotation_nodes(a: &mut Allocator) -> Vec<Node> {
        let mut nodes = Vec::new();
        let g = a.make_gaiji("X", None, false);
        render_into(a.gaiji(g), &mut nodes);
        for kind in [
            DirectiveKind::WarichuOpen,
            DirectiveKind::WarichuClose,
            DirectiveKind::NonCanonical,
            DirectiveKind::Editorial,
        ] {
            let p = a.make_directive("［＃注］", kind);
            render_into(a.annotation(p), &mut nodes);
        }
        let editor_note = a.make_directive("［＃入力者注(1)］", DirectiveKind::EditorNote);
        render_into(a.annotation(editor_note), &mut nodes);
        for (kind, raw) in [
            (DirectiveKind::RubyAttached, "［＃「親」にルビ］"),
            (DirectiveKind::RubyRetarget, "［＃ルビは「親」にかかる］"),
            (DirectiveKind::RubyPairOpen, "［＃左にルビ付き］"),
            (
                DirectiveKind::RubyPairClose,
                "［＃左に「よみ」のルビ付き終わり］",
            ),
        ] {
            let p = a.make_directive(raw, kind);
            render_into(a.annotation(p), &mut nodes);
        }
        for (kind, raw) in [
            (DirectiveKind::MarginNotePairOpen, "［＃注記付き］"),
            (DirectiveKind::MarginNotePairOpen, "［＃左に注記付き］"),
            (
                DirectiveKind::MarginNotePairClose,
                "［＃「よみ」の注記付き終わり］",
            ),
            (
                DirectiveKind::MarginNotePairClose,
                "［＃左に「よみ」の注記付き終わり］",
            ),
        ] {
            let p = a.make_directive(raw, kind);
            render_into(a.annotation(p), &mut nodes);
        }

        nodes
    }

    fn content_nodes(a: &mut Allocator) -> Vec<Node> {
        let mut nodes = Vec::new();
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
        nodes
    }

    fn forward_nodes(a: &mut Allocator) -> Vec<Node> {
        let mut nodes = Vec::new();
        for attr in [
            ForwardAttr::Bold,
            ForwardAttr::Italic,
            ForwardAttr::SuperScript,
            ForwardAttr::SubScript,
            ForwardAttr::SmallScript(BoutenPosition::Right),
            ForwardAttr::SmallScript(BoutenPosition::Left),
            ForwardAttr::Framed(EnclosureKind::Rule),
            ForwardAttr::Framed(EnclosureKind::Box),
            ForwardAttr::Framed(EnclosureKind::Circle),
            ForwardAttr::Framed(EnclosureKind::CircleDotted),
            ForwardAttr::Framed(EnclosureKind::DoubleRule),
            ForwardAttr::Horizontal,
            ForwardAttr::Caption,
            ForwardAttr::Fraction,
            ForwardAttr::FontSizeAbsolute(AbsoluteSize::ExtraLarge),
            ForwardAttr::FontSizeAbsolute(AbsoluteSize::Large),
            ForwardAttr::FontSizeAbsolute(AbsoluteSize::Medium),
            ForwardAttr::FontSizeAbsolute(AbsoluteSize::Small),
            ForwardAttr::FontSize(fs(1)),
            ForwardAttr::AlignEnd { offset: 1 },
        ] {
            let t = a.content_plain("強");
            render_into(
                a.forward_format(attr, t, ForwardOrigin::Reclaimed),
                &mut nodes,
            );
        }
        nodes
    }

    fn accent_nodes(a: &mut Allocator) -> Vec<Node> {
        let mut nodes = Vec::new();
        let dotted = a.content_plain("Sam");
        render_into(
            a.accent_dot(dotted, "mは上ドット付き", ForwardOrigin::Reclaimed),
            &mut nodes,
        );
        let accented = a.content_plain("e");
        render_into(
            a.forward_format(
                ForwardAttr::Accent(AccentMark::Acute),
                accented,
                ForwardOrigin::Reclaimed,
            ),
            &mut nodes,
        );
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

        nodes
    }

    fn indent_containers() -> [RegionFormat; 5] {
        [
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
            RegionFormat::Indent(IndentBlock {
                amount: 4,
                wrap: None,
                center: false,
                layout: IndentLayout::None,
                styles: BlockStyles {
                    gothic: true,
                    horizontal: true,
                    framed: true,
                    font: Some(FontShift(NonZeroI8::new(-1).unwrap())),
                },
            }),
        ]
    }

    fn class_containers() -> Vec<RegionFormat> {
        let mut containers = indent_containers().to_vec();
        containers.extend([
            RegionFormat::Warichu,
            RegionFormat::Framed(EnclosureKind::Rule),
            RegionFormat::AlignEnd { offset: 0 },
            RegionFormat::AlignEnd { offset: 2 },
            RegionFormat::LineWidth(lw(30)),
            RegionFormat::Bold { padded: false },
            RegionFormat::Bold { padded: true },
            RegionFormat::Gothic { padded: false },
            RegionFormat::Gothic { padded: true },
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
        ]);
        for &kind in BOUTEN_KINDS {
            for position in [
                BoutenPosition::Right,
                BoutenPosition::Left,
                BoutenPosition::Both,
            ] {
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
        containers
    }

    #[test]
    fn class_list_matches_emitted() {
        let listed: BTreeSet<String> = AOZORA_CLASSES.iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(
            all_emitted_classes(),
            listed,
            "AOZORA_CLASSES is out of sync with the renderer's emitted classes"
        );

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

    #[test]
    fn canonical_stylesheet_matches_emitted_classes() {
        const CSS: &str = include_str!("../../assets/aozora-notation.css");

        let styled = extract_css_classes(CSS);
        let scope: BTreeSet<String> = ["aozora-notation", "aozora-vertical"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let emitted: BTreeSet<String> = AOZORA_CLASSES.iter().map(|s| (*s).to_owned()).collect();

        for hook in &scope {
            assert!(
                styled.contains(hook),
                "canonical stylesheet must use the consumer scope hook `.{hook}`",
            );
        }

        let notation: BTreeSet<String> = styled.difference(&scope).cloned().collect();

        let dead: Vec<&String> = notation.difference(&emitted).collect();
        assert!(
            dead.is_empty(),
            "canonical stylesheet references classes the renderer never emits \
             (dead / typo'd): {dead:?}",
        );

        let missing: Vec<&String> = emitted.difference(&notation).collect();
        assert!(
            missing.is_empty(),
            "canonical stylesheet is missing a rule for emitted classes: {missing:?}",
        );
    }

    fn extract_css_classes(css: &str) -> BTreeSet<String> {
        let mut cleaned = String::with_capacity(css.len());
        let mut chars = css.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '/' && chars.peek() == Some(&'*') {
                chars.next();
                let mut prev = '\0';
                for d in chars.by_ref() {
                    if prev == '*' && d == '/' {
                        break;
                    }
                    prev = d;
                }
            } else {
                cleaned.push(c);
            }
        }

        let bytes = cleaned.as_bytes();
        let mut out = BTreeSet::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'.' {
                let start = i + 1;
                let mut j = start;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'-') {
                    j += 1;
                }
                let ident = &cleaned[start..j];
                if ident
                    .strip_prefix("aozora-")
                    .is_some_and(|rest| !rest.is_empty())
                {
                    out.insert(normalize_numeric_variant(ident));
                }
                i = j.max(i + 1);
            } else {
                i += 1;
            }
        }
        out
    }

    fn normalize_numeric_variant(cls: &str) -> String {
        if let Some(idx) = cls.rfind('-') {
            let suffix = &cls[idx + 1..];
            if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
                return cls[..idx].to_owned();
            }
        }
        cls.to_owned()
    }
}
