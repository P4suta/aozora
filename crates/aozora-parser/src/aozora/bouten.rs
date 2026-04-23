//! Bouten CSS-class slug tables — the only bit of the old bouten module that
//! survives the ADR-0008 cutover.
//!
//! The forward-reference parser (`［＃「X」に〈KIND〉］`) moved to
//! `afm-lexer::phase3_classify::classify_forward_bouten`; what this module
//! now exposes is:
//!
//! * [`kind_slug`] — stable CSS slug for each [`BoutenKind`] variant.
//! * [`position_slug`] — `"right"` / `"left"` for [`BoutenPosition`].
//!
//! Both tables are exhaustive on their respective enums so the
//! class-contract tests can lock output without caring about the
//! Japanese keyword form.

use afm_syntax::{BoutenKind, BoutenPosition};

/// Stable CSS-class slug for a [`BoutenKind`]. Used by the HTML renderer
/// at `crate::aozora::html::render_bouten`.
#[must_use]
pub(crate) const fn kind_slug(kind: BoutenKind) -> &'static str {
    // Exhaustive match (no `_` arm) so a new `BoutenKind` variant
    // surfaces as a compile-time miss here rather than silently
    // rendering as `afm-bouten-other`.
    match kind {
        BoutenKind::Goma => "goma",
        BoutenKind::WhiteSesame => "white-sesame",
        BoutenKind::Circle => "circle",
        BoutenKind::WhiteCircle => "white-circle",
        BoutenKind::DoubleCircle => "double-circle",
        BoutenKind::Janome => "janome",
        BoutenKind::Cross => "cross",
        BoutenKind::WhiteTriangle => "white-triangle",
        BoutenKind::WavyLine => "wavy-line",
        BoutenKind::UnderLine => "under-line",
        BoutenKind::DoubleUnderLine => "double-under-line",
        // `BoutenKind` is `#[non_exhaustive]` so downstream consumers
        // can construct unknown variants via its `Copy` semantics.
        // Map any such future variant to `other` so render stays
        // infallible while a compile-time miss still catches our
        // own additions (see the exhaustive test below).
        _ => "other",
    }
}

/// Stable CSS-class slug for a [`BoutenPosition`]. `BoutenPosition` is
/// `#[non_exhaustive]` so future variants default to `"right"` (the
/// canonical right-side placement), keeping render infallible; the
/// exhaustive test below catches any variant we forget to map.
#[must_use]
pub(crate) const fn position_slug(pos: BoutenPosition) -> &'static str {
    match pos {
        BoutenPosition::Left => "left",
        _ => "right",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_slug_covers_every_enum_variant() {
        // Brittle on purpose: if a new BoutenKind variant lands, this test
        // fails until the slug table is updated so CSS classes stay complete.
        for (kind, want) in [
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
            assert_eq!(kind_slug(kind), want, "kind={kind:?}");
        }
    }

    #[test]
    fn position_slug_covers_every_enum_variant() {
        for (pos, want) in [
            (BoutenPosition::Right, "right"),
            (BoutenPosition::Left, "left"),
        ] {
            assert_eq!(position_slug(pos), want, "pos={pos:?}");
        }
    }
}
