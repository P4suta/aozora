//! The CSS class names the HTML renderer can emit.
//!
//! [`AOZORA_CLASSES`] is the authoritative, public list of every
//! `aozora-*` class [`crate::render_node`] / [`crate::html`] writes.
//! Downstream consumers (e.g. the sibling `afm` crate) import it instead
//! of hand-mirroring the contract. The `class_list_matches_emitted`
//! test renders every `AozoraNode` + `ContainerKind` variant and asserts
//! the emitted class tokens are *exactly* this set, so the published
//! list can never drift from the emit sites.

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
    "aozora-annotation",
    "aozora-bold",
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
    "aozora-caption",
    "aozora-center",
    "aozora-container",
    "aozora-container-align-end",
    "aozora-container-bold",
    "aozora-container-center",
    "aozora-container-columns",
    "aozora-container-font-larger",
    "aozora-container-font-smaller",
    "aozora-container-horizontal",
    "aozora-container-indent",
    "aozora-container-italic",
    "aozora-container-keigakomi",
    "aozora-container-line-width",
    "aozora-container-table",
    "aozora-container-warichu",
    "aozora-container-wrap-indent",
    "aozora-font-larger",
    "aozora-font-smaller",
    "aozora-gaiji",
    "aozora-heading",
    "aozora-heading-hint",
    "aozora-heading-large",
    "aozora-heading-medium",
    "aozora-heading-same-line",
    "aozora-heading-small",
    "aozora-heading-window",
    "aozora-horizontal",
    "aozora-indent",
    "aozora-italic",
    "aozora-kaeriten",
    "aozora-keigakomi-inline",
    "aozora-kogaki-left",
    "aozora-kogaki-right",
    "aozora-page-break",
    "aozora-ruby-left",
    "aozora-sashie",
    "aozora-section-break",
    "aozora-section-break-kaicho",
    "aozora-section-break-kaidan",
    "aozora-section-break-kaimihiraki",
    "aozora-sidenote",
    "aozora-subscript",
    "aozora-superscript",
    "aozora-tcy",
    "aozora-warichu",
];

#[cfg(test)]
mod tests {
    use super::AOZORA_CLASSES;
    use crate::render_node::render;
    use aozora_syntax::alloc::BorrowedAllocator;
    use aozora_syntax::borrowed::{AozoraNode, Arena};
    use aozora_syntax::{
        AlignEnd, AnnotationKind, AozoraHeadingKind, AozoraHeadingStyle, BOUTEN_KINDS,
        BoutenPosition, Center, Container, ContainerKind, EmphasisKind, Indent, SectionKind,
    };
    use std::collections::BTreeSet;

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

    fn render_into(node: AozoraNode<'_>, set: &mut BTreeSet<String>) {
        let mut s = String::new();
        render(node, true, &mut s).expect("render into String is infallible");
        render(node, false, &mut s).expect("render into String is infallible");
        collect_classes(&s, set);
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one render call per AozoraNode + ContainerKind variant; \
                  splitting would scatter the exhaustive enumeration"
    )]
    fn class_list_matches_emitted() {
        let arena = Arena::new();
        let mut a = BorrowedAllocator::new(&arena);
        let mut emitted: BTreeSet<String> = BTreeSet::new();

        // --- leaf nodes ---
        render_into(a.page_break(), &mut emitted);
        render_into(a.kaeriten("一"), &mut emitted);
        render_into(a.center(Center { page: true }), &mut emitted);
        render_into(a.center(Center { page: false }), &mut emitted);
        render_into(a.indent(Indent { amount: 2 }), &mut emitted);
        render_into(a.align_end(AlignEnd { offset: 0 }), &mut emitted);
        render_into(a.align_end(AlignEnd { offset: 2 }), &mut emitted);
        render_into(a.sashie("f.png", None, None, None), &mut emitted);
        render_into(a.sashie_general("f.png", "図", None), &mut emitted);
        render_into(
            a.heading_hint(1, AozoraHeadingStyle::Standard, "x"),
            &mut emitted,
        );

        for k in [
            SectionKind::Kaicho,
            SectionKind::Kaidan,
            SectionKind::Kaimihiraki,
        ] {
            render_into(a.section_break(k), &mut emitted);
        }

        let g = a.make_gaiji("X", None, None);
        render_into(a.gaiji(g), &mut emitted);
        for kind in [
            AnnotationKind::WarichuOpen,
            AnnotationKind::WarichuClose,
            AnnotationKind::Unknown,
        ] {
            let p = a.make_annotation("［＃注］", kind);
            render_into(a.annotation(p), &mut emitted);
        }

        // Nodes needing Content. `content_plain` takes `&mut self` but
        // returns a Content borrowing the arena (not the allocator), so
        // the `&mut` borrow ends before each `&self` constructor runs and
        // plain sequential lets compile.
        let ruby_base = a.content_plain("親");
        let ruby_reading = a.content_plain("おや");
        render_into(a.ruby(ruby_base, ruby_reading, true), &mut emitted);
        let lruby_base = a.content_plain("子");
        let lruby_reading = a.content_plain("こ");
        render_into(a.left_ruby(lruby_base, lruby_reading), &mut emitted);
        let note_base = a.content_plain("孫");
        let note_text = a.content_plain("注");
        render_into(a.side_note(note_base, note_text), &mut emitted);
        let tcy = a.content_plain("囲");
        render_into(a.tate_chu_yoko(tcy, false), &mut emitted);
        let angle = a.content_plain("内");
        render_into(a.angle_quote(angle), &mut emitted);
        for &kind in BOUTEN_KINDS {
            for pos in [BoutenPosition::Right, BoutenPosition::Left] {
                let t = a.content_plain("文");
                render_into(a.bouten(kind, t, pos, false), &mut emitted);
            }
        }
        for kind in [
            EmphasisKind::Bold,
            EmphasisKind::Italic,
            EmphasisKind::SuperScript,
            EmphasisKind::SubScript,
            EmphasisKind::SmallRight,
            EmphasisKind::SmallLeft,
            EmphasisKind::KeigakomiInline,
            EmphasisKind::HorizontalInline,
            EmphasisKind::Caption,
            EmphasisKind::FontSize { steps: 2 },
            EmphasisKind::FontSize { steps: -2 },
        ] {
            let t = a.content_plain("強");
            render_into(a.emphasis(kind, t, false), &mut emitted);
        }
        for kind in [
            AozoraHeadingKind::Large,
            AozoraHeadingKind::Medium,
            AozoraHeadingKind::Small,
        ] {
            for style in [
                AozoraHeadingStyle::Standard,
                AozoraHeadingStyle::SameLine,
                AozoraHeadingStyle::Window,
            ] {
                let t = a.content_plain("見");
                render_into(a.aozora_heading(kind, style, t), &mut emitted);
            }
        }

        // --- containers (open + close) ---
        let mut containers = vec![
            ContainerKind::Indent {
                amount: 2,
                wrap: None,
                center: false,
            },
            ContainerKind::Indent {
                amount: 2,
                wrap: Some(4),
                center: true,
            },
            ContainerKind::Warichu,
            ContainerKind::Keigakomi,
            ContainerKind::AlignEnd { offset: 0 },
            ContainerKind::AlignEnd { offset: 2 },
            ContainerKind::LineWidth { width: 30 },
            ContainerKind::Bold { block: false },
            ContainerKind::Bold { block: true },
            ContainerKind::Italic { block: false },
            ContainerKind::Italic { block: true },
            ContainerKind::Columns { count: 2 },
            ContainerKind::Table,
            ContainerKind::Horizontal,
            ContainerKind::FontSize { steps: 2 },
            ContainerKind::FontSize { steps: -2 },
            ContainerKind::SmallScript {
                side: BoutenPosition::Right,
            },
            ContainerKind::SmallScript {
                side: BoutenPosition::Left,
            },
            ContainerKind::Caption { block: false },
            ContainerKind::Caption { block: true },
            ContainerKind::TcyRange,
        ];
        for &kind in BOUTEN_KINDS {
            for position in [BoutenPosition::Right, BoutenPosition::Left] {
                containers.push(ContainerKind::BoutenRange { kind, position });
            }
        }
        for kind in [
            AozoraHeadingKind::Large,
            AozoraHeadingKind::Medium,
            AozoraHeadingKind::Small,
        ] {
            for style in [
                AozoraHeadingStyle::Standard,
                AozoraHeadingStyle::SameLine,
                AozoraHeadingStyle::Window,
            ] {
                containers.push(ContainerKind::Heading {
                    kind,
                    style,
                    block: true,
                });
            }
        }
        for kind in containers {
            render_into(a.container(Container { kind }), &mut emitted);
        }

        let listed: BTreeSet<String> = AOZORA_CLASSES.iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(
            emitted, listed,
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
}
