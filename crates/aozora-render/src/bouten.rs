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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_slug_covers_every_bouten_variant() {
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
            assert_eq!(kind_slug(kind), slug, "kind_slug mismatch for {kind:?}");
        }
    }

    #[test]
    fn position_slug_maps_left_and_right() {
        assert_eq!(position_slug(BoutenPosition::Left), "left");
        assert_eq!(position_slug(BoutenPosition::Right), "right");
    }
}
