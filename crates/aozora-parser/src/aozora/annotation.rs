//! `［＃...］` inline-annotation scanner and keyword dispatcher.
//!
//! Pulled out of `adapter.rs::parse_bracket_annotation` so future recogniser
//! work (`PageBreak` promotion in C2, `Bouten` forward-reference in C3,
//! 縦中横 in C6, indent-leaf in C7) can extend the dispatch table in one
//! place without touching the inline dispatcher.
//!
//! This module intentionally does two things:
//!
//! 1. **Scan** a `［＃...］` span starting at the head of an input slice, and
//!    return the consumed byte count plus the bracket-interior body (between
//!    `＃` and `］`).
//! 2. **Classify** the body into an [`AozoraNode`] variant by keyword match.
//!    Until C2+ land, every classification is [`AnnotationKind::Unknown`] —
//!    the Tier-A invariant (no bare `［＃` leaks) already depends on *scan
//!    success*, not on *semantic classification*, so M0 behaviour is
//!    preserved byte-identical through this refactor.

use afm_syntax::{Annotation, AnnotationKind, AozoraNode, SectionKind};

/// Result of a successful `［＃...］` scan.
pub(crate) struct BracketMatch {
    /// The constructed AST node (classified per `classify`).
    pub node: AozoraNode,
    /// Total bytes consumed from the start of the input slice (includes the
    /// leading `［`, `＃`, the interior, and the closing `］`).
    pub consumed: usize,
}

/// Try to parse a `［＃...］` span starting at the head of `head`. Returns
/// `None` if:
/// - `head` doesn't begin with `［`,
/// - the `＃` is absent (`head` starts with `［` but no `＃` follows — lone
///   bracket falls through to comrak's default text handling),
/// - or no closing `］` is found (malformed sequence, leave as text for
///   graceful degradation).
#[must_use]
pub(crate) fn scan_bracket(head: &str) -> Option<BracketMatch> {
    let after_open = head.get('［'.len_utf8()..)?;
    if !after_open.starts_with('＃') {
        return None;
    }
    let body_start = '［'.len_utf8() + '＃'.len_utf8();
    let rest = head.get(body_start..)?;
    let close_relative = rest.find('］')?;
    let body = &rest[..close_relative];
    let total = body_start + close_relative + '］'.len_utf8();
    let raw = &head[..total];
    let node = classify(body, raw);
    Some(BracketMatch {
        node,
        consumed: total,
    })
}

/// Dispatch the bracket-body content to a concrete [`AozoraNode`].
///
/// Extended incrementally per M1 phase C:
/// - C2 (this commit): `改ページ` / `改丁` / `改段` / `改見開き` → `PageBreak`
///   / `SectionBreak` variants.
/// - C3 — C7 will add bouten, 縦中横, 字下げ, 地付き, and forward-reference
///   heading / bouten classifications.
///
/// Unknown bodies fall back to [`AnnotationKind::Unknown`] so the Tier-A
/// invariant (no bare `［＃` leaks) remains true throughout — graceful
/// degradation is an architectural guarantee (ADR-0003 §6).
fn classify(body: &str, raw: &str) -> AozoraNode {
    match body {
        "改ページ" => AozoraNode::PageBreak,
        "改丁" => AozoraNode::SectionBreak(SectionKind::Choho),
        "改段" => AozoraNode::SectionBreak(SectionKind::Dan),
        "改見開き" => AozoraNode::SectionBreak(SectionKind::Spread),
        _ => AozoraNode::Annotation(Annotation {
            raw: raw.into(),
            kind: AnnotationKind::Unknown,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_break_classifies_to_dedicated_variant() {
        let m = scan_bracket("［＃改ページ］続き").expect("scan");
        assert_eq!(m.consumed, "［＃改ページ］".len());
        assert!(
            matches!(m.node, AozoraNode::PageBreak),
            "expected PageBreak, got {:?}",
            m.node
        );
    }

    #[test]
    fn section_breaks_classify_per_kind() {
        use afm_syntax::SectionKind;
        for (body, want) in [
            ("改丁", SectionKind::Choho),
            ("改段", SectionKind::Dan),
            ("改見開き", SectionKind::Spread),
        ] {
            let input = format!("［＃{body}］");
            let m = scan_bracket(&input).expect("scan");
            assert_eq!(m.consumed, input.len());
            match m.node {
                AozoraNode::SectionBreak(got) => assert_eq!(got, want, "body={body}"),
                other => panic!("body={body}: expected SectionBreak({want:?}), got {other:?}"),
            }
        }
    }

    #[test]
    fn unknown_bodies_stay_as_unknown_annotation() {
        // Preserves Tier-A graceful degradation: bodies we don't yet classify
        // still get consumed and wrapped, never leak.
        let m = scan_bracket("［＃「可哀想」に傍点］あと").expect("scan");
        assert_eq!(m.consumed, "［＃「可哀想」に傍点］".len());
        let AozoraNode::Annotation(a) = &m.node else {
            panic!(
                "expected Annotation for unclassified body, got {:?}",
                m.node
            );
        };
        assert_eq!(&*a.raw, "［＃「可哀想」に傍点］");
        assert_eq!(a.kind, AnnotationKind::Unknown);
    }

    #[test]
    fn rejects_lone_open_bracket_without_hash() {
        // ［X］ has no ＃ after [, must fall through.
        assert!(scan_bracket("［X］").is_none());
    }

    #[test]
    fn rejects_open_bracket_without_close() {
        // Malformed — no ］ in the rest of the input.
        assert!(scan_bracket("［＃unclosed").is_none());
    }

    #[test]
    fn rejects_non_bracket_input() {
        assert!(scan_bracket("plain").is_none());
        assert!(scan_bracket("《ruby》").is_none());
    }
}
