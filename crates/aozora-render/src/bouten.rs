//! Bouten CSS-class slug tables (mirror of `aozora_parser::aozora::bouten`).
//!
//! Same exhaustive map of [`BoutenKind`]/[`BoutenPosition`] enum values
//! to the stable CSS slugs used by the bundled stylesheets. Lives here
//! so the borrowed-AST renderer (Plan B.3) does not need to depend on
//! `aozora-parser`'s internal `aozora` module.

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
