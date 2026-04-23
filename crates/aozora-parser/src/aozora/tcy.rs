//! 縦中横 (tate-chu-yoko) recognisers.
//!
//! Aozora Bunko describes two forms:
//!
//! - **Forward-reference**: `XX［＃「XX」は縦中横］` — the target literal is
//!   named inside the bracket and must also appear in the preceding inline
//!   text. Mirrors the bouten forward-reference shape but uses `は`
//!   (topic marker) in place of `に` (dative). This is the common form
//!   seen in real corpora (e.g. `Ｂ29［＃「29」は縦中横］`).
//!
//! - **Paired**: `［＃縦中横］content［＃縦中横終わり］` — the wrapper markers
//!   delimit a short inline run that should render as horizontal-in-vertical
//!   text. The inner span is kept short in practice (a couple of digits or
//!   a short gaiji body).
//!
//! Both promote to [`AozoraNode::TateChuYoko`]. The HTML renderer already
//! emits `<span class="afm-tcy">text</span>`; C6 only touches the scanner.

use afm_syntax::{AozoraNode, TateChuYoko};

use crate::aozora::annotation::BracketMatch;

/// Parse forward-reference tate-chu-yoko body `「X」は縦中横`. Returns the
/// target literal on success, or `None` if the body doesn't match.
///
/// The classifier caller is responsible for verifying the target also
/// appears in the preceding inline text before promoting.
#[must_use]
pub(crate) fn parse_forward_ref(body: &str) -> Option<&str> {
    let after_open = body.strip_prefix('「')?;
    let close = after_open.find('」')?;
    let target = &after_open[..close];
    if target.is_empty() {
        return None;
    }
    let after_close = &after_open[close + '」'.len_utf8()..];
    if after_close == "は縦中横" {
        Some(target)
    } else {
        None
    }
}

/// Maximum bytes between a `［＃縦中横］` open marker and its matching close.
/// 60 bytes comfortably covers the typical 2–6-character inner spans seen in
/// Aozora texts (Kanji + digits + short gaiji bodies) without letting a
/// dropped close marker swallow a whole paragraph.
const MAX_PAIRED_INNER_BYTES: usize = 60;
const PAIRED_CLOSE: &str = "［＃縦中横終わり］";

/// Given `head` starting at the candidate `［` and `open_consumed` bytes
/// already consumed for `［＃縦中横］`, try to find a matching close marker
/// within the configured look-ahead and return a [`BracketMatch`] for the
/// full paired span. Returns `None` if no close is found in range or the
/// inner span would include a nested bracket / ruby / newline.
#[must_use]
pub(crate) fn try_parse_paired(head: &str, open_consumed: usize) -> Option<BracketMatch> {
    let rest = head.get(open_consumed..)?;
    let search_limit = rest.len().min(MAX_PAIRED_INNER_BYTES + PAIRED_CLOSE.len());
    let window = rest.get(..search_limit)?;
    let close_rel = window.find(PAIRED_CLOSE)?;
    let inner = &rest[..close_rel];
    if inner.is_empty() || inner.contains('\n') || inner.contains('［') || inner.contains('《') {
        return None;
    }
    let consumed = open_consumed + close_rel + PAIRED_CLOSE.len();
    Some(BracketMatch {
        node: AozoraNode::TateChuYoko(TateChuYoko { text: inner.into() }),
        consumed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_ref_extracts_digit_target() {
        assert_eq!(parse_forward_ref("「29」は縦中横"), Some("29"));
    }

    #[test]
    fn forward_ref_extracts_kanji_target() {
        assert_eq!(parse_forward_ref("「左眼」は縦中横"), Some("左眼"));
    }

    #[test]
    fn forward_ref_rejects_missing_quotes() {
        assert!(parse_forward_ref("29は縦中横").is_none());
        assert!(parse_forward_ref("「29は縦中横").is_none());
    }

    #[test]
    fn forward_ref_rejects_wrong_particle() {
        assert!(parse_forward_ref("「29」に縦中横").is_none());
        assert!(parse_forward_ref("「29」が縦中横").is_none());
    }

    #[test]
    fn forward_ref_rejects_empty_target() {
        assert!(parse_forward_ref("「」は縦中横").is_none());
    }

    #[test]
    fn forward_ref_rejects_unknown_keyword() {
        assert!(parse_forward_ref("「X」は太字").is_none());
    }

    #[test]
    fn paired_consumes_open_inner_close() {
        let head = "［＃縦中横］20［＃縦中横終わり］後";
        let open_consumed = "［＃縦中横］".len();
        let m = try_parse_paired(head, open_consumed).expect("paired");
        let AozoraNode::TateChuYoko(t) = &m.node else {
            panic!("expected TateChuYoko, got {:?}", m.node);
        };
        assert_eq!(&*t.text, "20");
        assert_eq!(m.consumed, "［＃縦中横］20［＃縦中横終わり］".len());
    }

    #[test]
    fn paired_declines_when_close_marker_absent() {
        let head = "［＃縦中横］20と続く";
        let open_consumed = "［＃縦中横］".len();
        assert!(try_parse_paired(head, open_consumed).is_none());
    }

    #[test]
    fn paired_declines_when_close_marker_beyond_window() {
        // Fabricate a long ASCII inner to push the close beyond the 60-byte cap.
        let filler = "a".repeat(200);
        let head = format!("［＃縦中横］{filler}［＃縦中横終わり］");
        let open_consumed = "［＃縦中横］".len();
        assert!(try_parse_paired(&head, open_consumed).is_none());
    }

    #[test]
    fn paired_declines_when_inner_contains_nested_bracket() {
        let head = "［＃縦中横］a［＃x］［＃縦中横終わり］";
        let open_consumed = "［＃縦中横］".len();
        assert!(try_parse_paired(head, open_consumed).is_none());
    }

    #[test]
    fn paired_declines_on_empty_inner() {
        let head = "［＃縦中横］［＃縦中横終わり］";
        let open_consumed = "［＃縦中横］".len();
        assert!(try_parse_paired(head, open_consumed).is_none());
    }
}
