//! Bouten CSS-class slug table — the only bit of the old bouten module that
//! survives the ADR-0008 cutover.
//!
//! The forward-reference parser (`［＃「X」に〈KIND〉］`) moved to
//! `afm-lexer::phase3_classify::classify_forward_bouten`; all this module
//! now exposes is the stable CSS slug the HTML renderer needs so the
//! class-contract test can lock output without caring about the Japanese
//! keyword form.

use afm_syntax::BoutenKind;

/// Stable CSS-class slug for a [`BoutenKind`]. Used by the HTML renderer
/// at `crate::aozora::html::render_bouten`.
#[must_use]
pub(crate) const fn kind_slug(kind: BoutenKind) -> &'static str {
    match kind {
        BoutenKind::Goma => "goma",
        BoutenKind::Circle => "circle",
        BoutenKind::WhiteCircle => "white-circle",
        BoutenKind::DoubleCircle => "double-circle",
        BoutenKind::Janome => "janome",
        BoutenKind::WavyLine => "wavy-line",
        BoutenKind::UnderLine => "under-line",
        _ => "other",
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
            (BoutenKind::Circle, "circle"),
            (BoutenKind::WhiteCircle, "white-circle"),
            (BoutenKind::DoubleCircle, "double-circle"),
            (BoutenKind::Janome, "janome"),
            (BoutenKind::WavyLine, "wavy-line"),
            (BoutenKind::UnderLine, "under-line"),
        ] {
            assert_eq!(kind_slug(kind), want, "kind={kind:?}");
        }
    }
}
