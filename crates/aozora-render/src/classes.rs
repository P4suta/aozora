//! The CSS class names the HTML renderer can emit.
//!
//! [`AOZORA_CLASSES`] is the authoritative, public list of every
//! `aozora-*` class [`crate::render_node`] / [`crate::html`] writes.
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
    "aozora-combine-upright",
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
    "aozora-container-line-kumi",
    "aozora-container-line-width",
    "aozora-container-table",
    "aozora-container-warichu",
    "aozora-container-wrap-indent",
    "aozora-directive",
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
    "aozora-illustration",
    "aozora-indent",
    "aozora-italic",
    "aozora-kaeriten",
    "aozora-keigakomi-inline",
    "aozora-kogaki-left",
    "aozora-kogaki-right",
    "aozora-margin-note",
    "aozora-page-break",
    "aozora-ruby-left",
    "aozora-section-break",
    "aozora-section-break-kaicho",
    "aozora-section-break-kaidan",
    "aozora-section-break-kaimihiraki",
    "aozora-subscript",
    "aozora-superscript",
    "aozora-warichu",
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
    use crate::render_node::render;
    use aozora_syntax::alloc::BorrowedAllocator;
    use aozora_syntax::borrowed::{Arena, Node};
    use aozora_syntax::{
        AlignEnd, BOUTEN_KINDS, BoutenKind, BoutenPosition, Center, Container, ContainerKind,
        DirectiveKind, EMPHASIS_KINDS, EmphasisKind, HEADING_KINDS, HEADING_STYLES, HeadingStyle,
        Indent, IndentLayout, MarginNoteKind, SECTION_KINDS,
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

    fn render_into(node: Node<'_>, set: &mut BTreeSet<String>) {
        let mut s = String::new();
        render(node, true, &mut s).expect("render into String is infallible");
        render(node, false, &mut s).expect("render into String is infallible");
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
        render_into(a.heading_hint(1, HeadingStyle::Standard, "x"), &mut emitted);

        for &k in SECTION_KINDS {
            render_into(a.section_break(k), &mut emitted);
        }

        let g = a.make_gaiji("X", None, None, false);
        render_into(a.gaiji(g), &mut emitted);
        for kind in [
            DirectiveKind::WarichuOpen,
            DirectiveKind::WarichuClose,
            DirectiveKind::Unknown,
        ] {
            let p = a.make_directive("［＃注］", kind);
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
        render_into(
            a.side_note(MarginNoteKind::Gloss, note_base, note_text),
            &mut emitted,
        );
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
        for &kind in EMPHASIS_KINDS {
            let t = a.content_plain("強");
            render_into(a.emphasis(kind, t, false), &mut emitted);
        }
        // `EMPHASIS_KINDS` carries one `FontSize` (positive → font-larger);
        // its negative magnitude (font-smaller) is the only other class the
        // variant produces, so exercise that sign explicitly.
        let smaller = a.content_plain("小");
        render_into(
            a.emphasis(EmphasisKind::FontSize { steps: -1 }, smaller, false),
            &mut emitted,
        );
        for &kind in HEADING_KINDS {
            for &style in HEADING_STYLES {
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
                layout: IndentLayout::None,
            },
            ContainerKind::Indent {
                amount: 2,
                wrap: Some(4),
                center: true,
                layout: IndentLayout::None,
            },
            // #78 line-layout compounds — exercise the new line-kumi class
            // (字詰め reuses the standalone line-width class).
            ContainerKind::Indent {
                amount: 3,
                wrap: None,
                center: false,
                layout: IndentLayout::Kumi {
                    lines: 1,
                    width: 20,
                },
            },
            ContainerKind::Indent {
                amount: 8,
                wrap: None,
                center: false,
                layout: IndentLayout::LineWidth(18),
            },
            ContainerKind::Warichu,
            ContainerKind::Framed,
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
            ContainerKind::CombineUprightRange,
        ];
        for &kind in BOUTEN_KINDS {
            for position in [BoutenPosition::Right, BoutenPosition::Left] {
                containers.push(ContainerKind::BoutenRange { kind, position });
            }
        }
        for &kind in HEADING_KINDS {
            for &style in HEADING_STYLES {
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
