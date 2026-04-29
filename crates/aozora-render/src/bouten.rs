//! Bouten CSS-class slug tables.
//!
//! Exhaustive map of [`BoutenKind`] / [`BoutenPosition`] enum values
//! to the stable CSS slugs used by the bundled stylesheets.

use aozora_syntax::{BoutenKind, BoutenPosition};

#[must_use]
pub(crate) const fn kind_slug(kind: BoutenKind) -> &'static str {
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
        // BoutenKind is `#[non_exhaustive]`; default future variants
        // to "other" so render stays infallible.
        _ => "other",
    }
}

#[must_use]
pub(crate) const fn position_slug(pos: BoutenPosition) -> &'static str {
    match pos {
        BoutenPosition::Left => "left",
        _ => "right",
    }
}
