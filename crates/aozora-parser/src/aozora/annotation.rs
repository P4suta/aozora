//! `［＃...］` inline-annotation scanner and keyword dispatcher.
//!
//! Pulled out of `adapter.rs::parse_bracket_annotation` so future recogniser
//! work (C6 縦中横, C7 indent-leaf, Phase D paired containers) can extend
//! the dispatch table in one place without touching the inline dispatcher.
//!
//! This module does two things:
//!
//! 1. **Scan** a `［＃...］` span at the head of an input slice and return the
//!    consumed byte count plus the bracket-interior body (between `＃` and `］`).
//! 2. **Classify** the body into an [`AozoraNode`] variant by keyword match.
//!    Unrecognised bodies degrade to [`AnnotationKind::Unknown`] so the
//!    Tier-A invariant (no bare `［＃` leaks) survives for any future corpus.

use afm_syntax::{Annotation, AnnotationKind, AozoraNode, Bouten, SectionKind, TateChuYoko};

use crate::aozora::bouten as bouten_mod;
use crate::aozora::layout as layout_mod;
use crate::aozora::tcy as tcy_mod;

/// Context passed to [`scan_bracket`].
///
/// Carries everything the classifier needs to resolve forward references
/// (currently just [`Bouten`]) without passing a grab-bag of positional
/// arguments. Kept small and `Copy` so the seam between `adapter.rs` and the
/// scanner stays cheap.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BracketCtx<'a> {
    /// Slice starting at the candidate `［`. The scanner only looks inside
    /// this slice; the caller is responsible for its bounds.
    pub(crate) head: &'a str,
    /// Text the inline parser has already committed on the current line —
    /// used by forward-reference classifiers (e.g. bouten) to check that
    /// the annotation's target literal is actually present before
    /// promoting.
    pub(crate) preceding: &'a str,
}

/// Result of a successful `［＃...］` scan.
pub(crate) struct BracketMatch {
    /// The constructed AST node (classified per `classify`).
    pub(crate) node: AozoraNode,
    /// Total bytes consumed from the start of the input slice (includes the
    /// leading `［`, `＃`, the interior, and the closing `］`).
    pub(crate) consumed: usize,
}

/// Try to parse a `［＃...］` span at the head of `cx.head`. Returns `None` if:
/// - the slice doesn't begin with `［`,
/// - the `＃` is absent (`［` but no `＃` follows — lone bracket falls
///   through to comrak's default text handling),
/// - or no closing `］` is found (malformed, leave as text for graceful
///   degradation).
#[must_use]
pub(crate) fn scan_bracket(cx: BracketCtx<'_>) -> Option<BracketMatch> {
    let after_open = cx.head.get('［'.len_utf8()..)?;
    if !after_open.starts_with('＃') {
        return None;
    }
    let body_start = '［'.len_utf8() + '＃'.len_utf8();
    let rest = cx.head.get(body_start..)?;
    let close_relative = rest.find('］')?;
    let body = &rest[..close_relative];
    let total = body_start + close_relative + '］'.len_utf8();

    // Paired 縦中横 has to be tried before the leaf classifier — a lone
    // `［＃縦中横］` is the *open* marker, and we need to consume past the
    // matching close. If the close isn't found in range we fall through to
    // the regular classifier, which emits Annotation{Unknown}.
    if body == "縦中横"
        && let Some(paired) = tcy_mod::try_parse_paired(cx.head, total)
    {
        return Some(paired);
    }

    let raw = &cx.head[..total];
    let node = classify(body, raw, &cx);
    Some(BracketMatch {
        node,
        consumed: total,
    })
}

/// Dispatch the bracket-body content to a concrete [`AozoraNode`].
///
/// Extended incrementally per M1 phase C:
/// - C2: `改ページ` / `改丁` / `改段` / `改見開き` → `PageBreak` / `SectionBreak`.
/// - C3: `「X」に{傍点,丸傍点,白丸傍点,二重丸傍点,蛇の目傍点,波線,傍線}` →
///   `Bouten` when `X` also appears in the preceding inline text.
/// - C6, C7 will add 縦中横 and leaf indent / 地付き classifications.
///
/// Unknown bodies fall back to [`AnnotationKind::Unknown`]. Graceful
/// degradation is an architectural guarantee (ADR-0003 §6).
fn classify(body: &str, raw: &str, cx: &BracketCtx<'_>) -> AozoraNode {
    match body {
        "改ページ" => AozoraNode::PageBreak,
        "改丁" => AozoraNode::SectionBreak(SectionKind::Choho),
        "改段" => AozoraNode::SectionBreak(SectionKind::Dan),
        "改見開き" => AozoraNode::SectionBreak(SectionKind::Spread),
        _ => try_leaf_indent(body)
            .or_else(|| try_leaf_align_end(body))
            .or_else(|| try_forward_ref_bouten(body, cx))
            .or_else(|| try_forward_ref_tcy(body, cx))
            .unwrap_or_else(|| {
                AozoraNode::Annotation(Annotation {
                    raw: raw.into(),
                    kind: AnnotationKind::Unknown,
                })
            }),
    }
}

/// Leaf `{N}字下げ` → [`AozoraNode::Indent`] (paired `［＃ここから…］` is
/// Phase D). Returns `None` if the body isn't the leaf indent shape.
fn try_leaf_indent(body: &str) -> Option<AozoraNode> {
    layout_mod::parse_indent(body).map(AozoraNode::Indent)
}

/// Leaf `地付き` / `地からN字上げ` → [`AozoraNode::AlignEnd`]. Returns
/// `None` if the body isn't one of those forms.
fn try_leaf_align_end(body: &str) -> Option<AozoraNode> {
    layout_mod::parse_align_end(body).map(AozoraNode::AlignEnd)
}

/// Attempt to promote `body` to a [`Bouten`] via the forward-reference
/// parser. Returns `None` unless the body is a forward-reference shape
/// **and** the named target literal is also present in the preceding
/// inline run — without that confirmation, wrapping the target in an
/// `<em>` at render time would invent emphasis that wasn't in the source.
/// On `None`, the caller emits `Annotation{Unknown}`.
fn try_forward_ref_bouten(body: &str, cx: &BracketCtx<'_>) -> Option<AozoraNode> {
    let frb = bouten_mod::parse_forward_ref(body)?;
    if !cx.preceding.contains(frb.target) {
        return None;
    }
    Some(AozoraNode::Bouten(Bouten {
        kind: frb.kind,
        target: frb.target.into(),
    }))
}

/// Attempt to promote `body` to a [`TateChuYoko`] via the forward-reference
/// parser (`「X」は縦中横`). Same contract as bouten: the target literal must
/// also appear in the preceding run. On `None`, fall back to
/// `Annotation{Unknown}`.
fn try_forward_ref_tcy(body: &str, cx: &BracketCtx<'_>) -> Option<AozoraNode> {
    let target = tcy_mod::parse_forward_ref(body)?;
    if !cx.preceding.contains(target) {
        return None;
    }
    Some(AozoraNode::TateChuYoko(TateChuYoko {
        text: target.into(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use afm_syntax::{BoutenKind, SectionKind};

    fn plain(head: &str) -> BracketCtx<'_> {
        BracketCtx {
            head,
            preceding: "",
        }
    }

    #[test]
    fn page_break_classifies_to_dedicated_variant() {
        let m = scan_bracket(plain("［＃改ページ］続き")).expect("scan");
        assert_eq!(m.consumed, "［＃改ページ］".len());
        assert!(
            matches!(m.node, AozoraNode::PageBreak),
            "expected PageBreak, got {:?}",
            m.node
        );
    }

    #[test]
    fn section_breaks_classify_per_kind() {
        for (body, want) in [
            ("改丁", SectionKind::Choho),
            ("改段", SectionKind::Dan),
            ("改見開き", SectionKind::Spread),
        ] {
            let input = format!("［＃{body}］");
            let m = scan_bracket(plain(&input)).expect("scan");
            assert_eq!(m.consumed, input.len());
            match m.node {
                AozoraNode::SectionBreak(got) => assert_eq!(got, want, "body={body}"),
                other => panic!("body={body}: expected SectionBreak({want:?}), got {other:?}"),
            }
        }
    }

    #[test]
    fn forward_ref_bouten_without_matching_preceding_falls_back_to_unknown() {
        // Empty preceding → target absent → Annotation{Unknown}. Tier-A still
        // holds because the bracket is still consumed.
        let m = scan_bracket(plain("［＃「可哀想」に傍点］あと")).expect("scan");
        assert_eq!(m.consumed, "［＃「可哀想」に傍点］".len());
        let AozoraNode::Annotation(a) = &m.node else {
            panic!("expected Annotation, got {:?}", m.node);
        };
        assert_eq!(&*a.raw, "［＃「可哀想」に傍点］");
        assert_eq!(a.kind, AnnotationKind::Unknown);
    }

    #[test]
    fn forward_ref_bouten_with_matching_preceding_promotes_to_bouten() {
        let m = scan_bracket(BracketCtx {
            head: "［＃「可哀想」に傍点］あと",
            preceding: "可哀想",
        })
        .expect("scan");
        let AozoraNode::Bouten(b) = &m.node else {
            panic!("expected Bouten, got {:?}", m.node);
        };
        assert_eq!(b.kind, BoutenKind::Goma);
        assert_eq!(b.target.as_plain().expect("plain"), "可哀想");
    }

    #[test]
    fn forward_ref_bouten_kind_keywords_all_round_trip() {
        let cases = [
            ("傍点", BoutenKind::Goma),
            ("丸傍点", BoutenKind::Circle),
            ("白丸傍点", BoutenKind::WhiteCircle),
            ("二重丸傍点", BoutenKind::DoubleCircle),
            ("蛇の目傍点", BoutenKind::Janome),
            ("波線", BoutenKind::WavyLine),
            ("傍線", BoutenKind::UnderLine),
        ];
        for (keyword, want) in cases {
            let head = format!("［＃「X」に{keyword}］");
            let m = scan_bracket(BracketCtx {
                head: &head,
                preceding: "X",
            })
            .expect("scan");
            let AozoraNode::Bouten(b) = &m.node else {
                panic!("keyword={keyword}: expected Bouten, got {:?}", m.node);
            };
            assert_eq!(b.kind, want, "keyword={keyword}");
            assert_eq!(
                b.target.as_plain().expect("plain"),
                "X",
                "keyword={keyword}"
            );
        }
    }

    #[test]
    fn forward_ref_bouten_unknown_kind_falls_back_to_unknown() {
        // 白ゴマ傍点 exists in Aozora but not in BoutenKind — degrade.
        let m = scan_bracket(BracketCtx {
            head: "［＃「X」に白ゴマ傍点］",
            preceding: "X",
        })
        .expect("scan");
        assert!(
            matches!(m.node, AozoraNode::Annotation(_)),
            "expected Annotation fallback, got {:?}",
            m.node
        );
    }

    #[test]
    fn rejects_lone_open_bracket_without_hash() {
        assert!(scan_bracket(plain("［X］")).is_none());
    }

    #[test]
    fn rejects_open_bracket_without_close() {
        assert!(scan_bracket(plain("［＃unclosed")).is_none());
    }

    #[test]
    fn rejects_non_bracket_input() {
        assert!(scan_bracket(plain("plain")).is_none());
        assert!(scan_bracket(plain("《ruby》")).is_none());
    }
}
