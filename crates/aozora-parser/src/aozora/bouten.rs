//! Forward-reference bouten (`［＃「X」に〈KIND〉］`) parser.
//!
//! The body of the annotation names the *target* literal — usually a run of
//! characters that already appeared in the preceding inline text — and the
//! *kind* of emphasis to apply to that run (default 傍点 = Goma, plus six
//! shaped variants covered by [`afm_syntax::BoutenKind`]).
//!
//! This module is pure: it extracts `(kind, target_literal)` from a body, and
//! resolves the target's most recent occurrence in the preceding text to a
//! source [`Span`]. Dispatch glue lives in `annotation::classify`.
//!
//! Paired bouten (`［＃傍点］…［＃傍点終わり］`) is a separate parse path landing
//! with the paired-block container hook in Phase D.
//!
//! # Kind keyword table
//!
//! | Japanese     | [`BoutenKind`]          |
//! |--------------|-------------------------|
//! | 傍点         | [`BoutenKind::Goma`]    |
//! | 丸傍点       | [`BoutenKind::Circle`]  |
//! | 白丸傍点     | [`BoutenKind::WhiteCircle`] |
//! | 二重丸傍点   | [`BoutenKind::DoubleCircle`] |
//! | 蛇の目傍点   | [`BoutenKind::Janome`]  |
//! | 波線         | [`BoutenKind::WavyLine`] |
//! | 傍線         | [`BoutenKind::UnderLine`] |
//!
//! Aozora Bunko ships further variants (白ゴマ / ばつ / 三角 / 二重傍線 / 破線 /
//! 鎖線 …); adding support is a matter of extending the `BoutenKind` enum and
//! this keyword table together.

use afm_syntax::{BoutenKind, Span};

/// Extracted shape of a forward-reference bouten body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ForwardRefBouten<'a> {
    pub kind: BoutenKind,
    /// The literal characters between the `「` and `」` in the annotation. The
    /// annotation is only promoted to a [`afm_syntax::Bouten`] when this
    /// literal is also present in the preceding inline text.
    pub target: &'a str,
}

/// Parse an annotation body of shape `「X」に〈KIND〉` into a
/// [`ForwardRefBouten`]. Returns `None` if the body doesn't match that shape
/// or the keyword isn't one we map; the caller falls through to an
/// `Annotation{Unknown}` wrapper so the Tier-A invariant (no bare `［＃`
/// leaks) is preserved.
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

/// Locate the last occurrence of `target` in `preceding` and translate its
/// byte range to a source [`Span`] given `line_start` — the byte offset of
/// `preceding[0]` in the source. Returns `None` if `target` is absent from
/// `preceding` (signals to dispatcher: demote to `Annotation{Unknown}`).
#[must_use]
pub(crate) fn resolve_target_span(target: &str, preceding: &str, line_start: u32) -> Option<Span> {
    let relative = preceding.rfind(target)?;
    let start = line_start.checked_add(u32::try_from(relative).ok()?)?;
    let end = start.checked_add(u32::try_from(target.len()).ok()?)?;
    Some(Span::new(start, end))
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
    fn resolves_target_to_last_occurrence() {
        let span = resolve_target_span("ab", "xabyab", 100).expect("resolved");
        assert_eq!(span.start, 100 + 4);
        assert_eq!(span.end, 100 + 4 + 2);
    }

    #[test]
    fn resolve_returns_none_when_target_absent() {
        assert!(resolve_target_span("foo", "bar baz", 0).is_none());
    }

    #[test]
    fn resolve_returns_none_when_preceding_empty() {
        assert!(resolve_target_span("foo", "", 0).is_none());
    }

    #[test]
    fn resolve_handles_multibyte_target() {
        let preceding = "前段可哀想";
        let target = "可哀想";
        let expected_start = "前段".len();
        let span = resolve_target_span(target, preceding, 0).expect("resolved");
        assert_eq!(span.start as usize, expected_start);
        assert_eq!(span.end as usize, expected_start + target.len());
    }
}
