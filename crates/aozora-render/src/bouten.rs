//! Bouten CSS-class slug tables.
//!
//! Exhaustive map of [`BoutenKind`] / [`BoutenPosition`] enum values
//! to the stable CSS slugs used by the bundled stylesheets.

use aozora_syntax::{BoutenKind, BoutenPosition};

#[must_use]
pub(crate) fn kind_slug(kind: BoutenKind) -> &'static str {
    // Single source of truth: the romaji slug lives in the spec slug
    // table (`RENDER_SLUGS`), keyed by the canonical 青空文庫 keyword.
    // `BoutenKind` is `#[non_exhaustive]`; an unknown kind keys the bare
    // 傍点 (→ `goma`), keeping render infallible — `unwrap_or` only
    // guards the theoretical lookup miss.
    aozora_spec::roman_slug(kind.keyword()).unwrap_or("other")
}

#[must_use]
pub(crate) const fn position_slug(pos: BoutenPosition) -> &'static str {
    match pos {
        BoutenPosition::Left => "left",
        _ => "right",
    }
}
