//! Forward-reference bouten (`［＃「X」に〈KIND〉］`) parser.
//!
//! The body of the annotation names the *target* literal — usually a run of
//! characters that already appeared in the preceding inline text — and the
//! *kind* of emphasis to apply to that run (default 傍点 = Goma, plus six
//! shaped variants covered by [`afm_syntax::BoutenKind`]).
//!
//! This module is pure: it extracts `(kind, target_literal)` from a body.
//! The classifier in [`crate::aozora::annotation`] then confirms the target
//! is present in the preceding inline run before promoting to a
//! [`afm_syntax::Bouten`]; absent targets degrade to `Annotation{Unknown}`
//! so the Tier-A invariant (no bare `［＃` leaks) always holds.
//!
//! Paired bouten (`［＃傍点］…［＃傍点終わり］`) is a separate parse path landing
//! with the paired-block container hook in Phase D.
//!
//! # Kind keyword table
//!
//! | Japanese     | [`BoutenKind`]              |
//! |--------------|-----------------------------|
//! | 傍点         | [`BoutenKind::Goma`]        |
//! | 丸傍点       | [`BoutenKind::Circle`]      |
//! | 白丸傍点     | [`BoutenKind::WhiteCircle`] |
//! | 二重丸傍点   | [`BoutenKind::DoubleCircle`]|
//! | 蛇の目傍点   | [`BoutenKind::Janome`]      |
//! | 波線         | [`BoutenKind::WavyLine`]    |
//! | 傍線         | [`BoutenKind::UnderLine`]   |
//!
//! Aozora Bunko ships further variants (白ゴマ / ばつ / 三角 / 二重傍線 / 破線 /
//! 鎖線 …); adding support is a matter of extending the `BoutenKind` enum and
//! this keyword table together.

use afm_syntax::BoutenKind;

/// Extracted shape of a forward-reference bouten body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ForwardRefBouten<'a> {
    pub kind: BoutenKind,
    /// The literal characters between the `「` and `」` in the annotation.
    /// The classifier caller only promotes the annotation to
    /// [`afm_syntax::Bouten`] when this literal also appears in the preceding
    /// inline text, so the HTML rendering can legitimately wrap it.
    pub target: &'a str,
}

/// Parse an annotation body of shape `「X」に〈KIND〉` into a
/// [`ForwardRefBouten`]. Returns `None` if the body doesn't match that shape
/// or the keyword isn't one we map.
#[must_use]
pub(crate) fn parse_forward_ref(body: &str) -> Option<ForwardRefBouten<'_>> {
    let after_open = body.strip_prefix('「')?;
    let close = after_open.find('」')?;
    let target = &after_open[..close];
    if target.is_empty() {
        return None;
    }
    let after_close = &after_open[close + '」'.len_utf8()..];
    let keyword = after_close.strip_prefix('に')?;
    let kind = classify_kind(keyword)?;
    Some(ForwardRefBouten { kind, target })
}

/// Map the keyword after `に` to a [`BoutenKind`]. Unknown keywords return
/// `None`; the caller degrades to `Annotation{Unknown}`.
#[must_use]
fn classify_kind(keyword: &str) -> Option<BoutenKind> {
    match keyword {
        "傍点" => Some(BoutenKind::Goma),
        "丸傍点" => Some(BoutenKind::Circle),
        "白丸傍点" => Some(BoutenKind::WhiteCircle),
        "二重丸傍点" => Some(BoutenKind::DoubleCircle),
        "蛇の目傍点" => Some(BoutenKind::Janome),
        "波線" => Some(BoutenKind::WavyLine),
        "傍線" => Some(BoutenKind::UnderLine),
        _ => None,
    }
}

/// Stable CSS-class slug for a [`BoutenKind`]. Used by the HTML renderer so
/// the class-contract test can lock the output without caring about the
/// Japanese keyword form.
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
    fn parses_default_goma_kind() {
        let frb = parse_forward_ref("「可哀想」に傍点").expect("goma");
        assert_eq!(frb.kind, BoutenKind::Goma);
        assert_eq!(frb.target, "可哀想");
    }

    #[test]
    fn parses_all_seven_enum_keyword_aliases() {
        let cases = [
            ("「X」に傍点", BoutenKind::Goma),
            ("「X」に丸傍点", BoutenKind::Circle),
            ("「X」に白丸傍点", BoutenKind::WhiteCircle),
            ("「X」に二重丸傍点", BoutenKind::DoubleCircle),
            ("「X」に蛇の目傍点", BoutenKind::Janome),
            ("「X」に波線", BoutenKind::WavyLine),
            ("「X」に傍線", BoutenKind::UnderLine),
        ];
        for (body, want) in cases {
            let frb =
                parse_forward_ref(body).unwrap_or_else(|| panic!("parse failed for {body:?}"));
            assert_eq!(frb.kind, want, "body={body:?}");
            assert_eq!(frb.target, "X");
        }
    }

    #[test]
    fn unknown_kind_keywords_decline() {
        // 白ゴマ傍点 is a real Aozora variant but not in BoutenKind yet; the
        // parser must decline so the caller can fall back to Unknown.
        assert!(parse_forward_ref("「X」に白ゴマ傍点").is_none());
        assert!(parse_forward_ref("「X」にばつ傍点").is_none());
    }

    #[test]
    fn missing_open_bracket_declines() {
        assert!(parse_forward_ref("X」に傍点").is_none());
    }

    #[test]
    fn missing_close_bracket_declines() {
        assert!(parse_forward_ref("「X に傍点").is_none());
    }

    #[test]
    fn empty_target_literal_declines() {
        assert!(parse_forward_ref("「」に傍点").is_none());
    }

    #[test]
    fn missing_ni_particle_declines() {
        assert!(parse_forward_ref("「X」傍点").is_none());
    }

    #[test]
    fn plain_keyword_body_declines() {
        // Non forward-reference body — should be handled by other classifiers.
        assert!(parse_forward_ref("改ページ").is_none());
    }

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
