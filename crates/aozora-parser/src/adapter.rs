//! Adapter implementing [`afm_syntax::AozoraExtension`] for the forked comrak.
//!
//! This is the concrete object registered on `comrak::ExtensionOptions::aozora`.
//! It holds no mutable state (so it's cheap to share via `Arc` across threads) and
//! dispatches inline / block / render callbacks to module-level functions under
//! `crate::aozora`.

use core::fmt;

use afm_syntax::{AozoraExtension, AozoraNode, BlockCtx, BlockMatch, InlineCtx, InlineMatch};

use crate::aozora::{self, annotation::BracketCtx};

/// Zero-state adapter. Construct once per parse session (or reuse globally) and
/// register via `Options::default().extension.aozora = Some(Arc::new(AfmAdapter));`.
#[derive(Debug, Default, Clone, Copy)]
pub struct AfmAdapter;

impl AozoraExtension for AfmAdapter {
    fn try_parse_inline(&self, cx: InlineCtx<'_>) -> Option<InlineMatch> {
        let head = cx.input.get(cx.pos..)?;
        match classify_inline_head(head) {
            InlineTrigger::Bar => parse_bar_ruby(head),
            InlineTrigger::OpenRuby => parse_implicit_ruby(head, cx.preceding),
            InlineTrigger::OpenBracket => parse_bracket_annotation(head, cx.preceding),
            InlineTrigger::ReferenceMark => parse_reference_mark(head, cx.preceding),
            InlineTrigger::None => None,
        }
    }

    fn try_start_block(&self, _cx: BlockCtx<'_>) -> BlockMatch {
        // M0 Spike: all ［＃...］ annotations are recognised inline. Paired block
        // annotations (ここから字下げ / ここで字下げ終わり, 割り注, 罫囲み) land in
        // a future commit with proper container-stack management.
        BlockMatch::NotOurs
    }

    fn render_html(&self, node: &AozoraNode, writer: &mut dyn fmt::Write) -> fmt::Result {
        aozora::html::render(node, writer)
    }
}

/// Characters that open Aozora inline constructs. The comrak-side dispatcher has
/// already filtered by lead byte (0xEF / 0xE3 / 0xE2); we refine here with the
/// full codepoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineTrigger {
    /// `｜` U+FF5C — explicit ruby base delimiter
    Bar,
    /// `《` U+300A — ruby reading open (implicit form, or bouten double-bracket)
    OpenRuby,
    /// `［` U+FF3B — block-annotation open (expects `＃` to follow)
    OpenBracket,
    /// `※` U+203B — reference mark (often followed by `［＃…］` for gaiji)
    ReferenceMark,
    None,
}

fn classify_inline_head(head: &str) -> InlineTrigger {
    let first = head.chars().next();
    match first {
        Some('｜') => InlineTrigger::Bar,
        Some('《') => InlineTrigger::OpenRuby,
        Some('［') => InlineTrigger::OpenBracket,
        Some('※') => InlineTrigger::ReferenceMark,
        _ => InlineTrigger::None,
    }
}

/// `｜<base>《<reading>》` — consume the bar, delegate to the ruby parser, then
/// return the merged consumption count (bar + inner span).
fn parse_bar_ruby(head: &str) -> Option<InlineMatch> {
    let bar_len = '｜'.len_utf8();
    let rest = head.get(bar_len..)?;
    let (ruby, inner) = aozora::ruby::parse(rest, true, "")?;
    InlineMatch::new(AozoraNode::Ruby(ruby), bar_len + inner)
}

/// `《<reading>》` with the base recovered from `preceding` — pure delegation.
/// Implicit form requires a trailing kanji run in `preceding`; otherwise decline.
fn parse_implicit_ruby(head: &str, preceding: &str) -> Option<InlineMatch> {
    let (ruby, consumed) = aozora::ruby::parse(head, false, preceding)?;
    InlineMatch::new(AozoraNode::Ruby(ruby), consumed)
}

/// `［＃...］` — scan to the matching `］` and dispatch the interior by
/// keyword via [`crate::aozora::annotation::scan_bracket`]. Returns `None` if
/// the `＃` is absent (lone `［` falls through to comrak's default text
/// handling) or no closing `］` is found (malformed sequence, leave as text
/// for graceful degradation).
///
/// Classification semantics live in `aozora::annotation`; this function is
/// only the adapter-side glue that converts a successful scan into an
/// [`InlineMatch`].
fn parse_bracket_annotation<'a>(head: &'a str, preceding: &'a str) -> Option<InlineMatch> {
    let m = aozora::annotation::scan_bracket(BracketCtx { head, preceding })?;
    InlineMatch::new(m.node, m.consumed)
}

/// `※` on its own is a normal character. Only when it precedes `［＃` does it
/// start a gaiji annotation; in that case consume the ※ + the bracket body.
fn parse_reference_mark<'a>(head: &'a str, preceding: &'a str) -> Option<InlineMatch> {
    let mark_len = '※'.len_utf8();
    let after_mark = head.get(mark_len..)?;
    if !after_mark.starts_with('［') {
        return None;
    }
    let m = parse_bracket_annotation(after_mark, preceding)?;
    InlineMatch::new(m.node, mark_len + m.consumed.get())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    fn adapter() -> AfmAdapter {
        AfmAdapter
    }

    fn ctx<'a>(input: &'a str, preceding: &'a str) -> InlineCtx<'a> {
        InlineCtx::new(input, 0, preceding)
    }

    #[test]
    fn rejects_plain_text() {
        assert!(adapter().try_parse_inline(ctx("hello", "")).is_none());
    }

    #[test]
    fn recognises_explicit_ruby() {
        let m = adapter()
            .try_parse_inline(ctx("｜青梅《おうめ》の", ""))
            .expect("ruby");
        assert!(matches!(m.node, AozoraNode::Ruby(_)));
        assert_eq!(m.consumed.get(), "｜青梅《おうめ》".len());
    }

    #[test]
    fn recognises_implicit_ruby_after_kanji() {
        let m = adapter()
            .try_parse_inline(ctx("《にほん》です", "彼は日本"))
            .expect("ruby");
        assert!(matches!(m.node, AozoraNode::Ruby(_)));
    }

    #[test]
    fn implicit_ruby_without_kanji_declined() {
        assert!(
            adapter()
                .try_parse_inline(ctx("《おうめ》", "ひらがな"))
                .is_none()
        );
    }

    #[test]
    fn recognises_page_break_bracket_annotation() {
        // ［＃改ページ］ promotes to the PageBreak variant (see C2).
        let m = adapter()
            .try_parse_inline(ctx("［＃改ページ］続き", ""))
            .expect("annot");
        assert!(matches!(m.node, AozoraNode::PageBreak));
        assert_eq!(m.consumed.get(), "［＃改ページ］".len());
    }

    #[test]
    fn unknown_bracket_annotation_stays_as_annotation_node() {
        let m = adapter()
            .try_parse_inline(ctx("［＃ほげふが］続き", ""))
            .expect("annot");
        assert!(matches!(m.node, AozoraNode::Annotation(_)));
    }

    #[test]
    fn forward_reference_bouten_without_matching_preceding_falls_back_to_annotation() {
        // With `preceding=""` the target "可哀想" can't be located → the
        // scanner degrades to `Annotation{Unknown}` so Tier-A (no bare ［＃
        // leaks) is still guaranteed. C3 introduces the promotion path; the
        // "promotes to Bouten" case is asserted end-to-end in
        // `preceding_kanji_run_promotes_forward_reference_bouten`.
        let input = "［＃「可哀想」に傍点］という気";
        let m = adapter()
            .try_parse_inline(ctx(input, ""))
            .expect("bracket must be consumed even when target doesn't resolve");
        let expected_len = "［＃「可哀想」に傍点］".len();
        assert_eq!(
            m.consumed.get(),
            expected_len,
            "consumed {} bytes, expected {expected_len}",
            m.consumed.get()
        );
        let AozoraNode::Annotation(a) = &m.node else {
            panic!("expected Annotation fallback, got {:?}", m.node);
        };
        assert_eq!(&*a.raw, "［＃「可哀想」に傍点］");
    }

    #[test]
    fn preceding_kanji_run_promotes_forward_reference_bouten() {
        // End-to-end through the full pipeline. With the target literal in
        // the preceding run, C3 must promote the annotation to Bouten and
        // drop the Annotation wrapper entirely.
        let nodes = test_support::collect_aozora("可哀想［＃「可哀想」に傍点］という気");
        let bouten_count = nodes
            .iter()
            .filter(|n| matches!(n, AozoraNode::Bouten(_)))
            .count();
        let annotation_count = nodes
            .iter()
            .filter(|n| matches!(n, AozoraNode::Annotation(_)))
            .count();
        assert_eq!(
            bouten_count, 1,
            "expected one promoted Bouten, got {nodes:?}"
        );
        assert_eq!(
            annotation_count, 0,
            "Annotation wrapper must disappear when bouten promotes; got {nodes:?}"
        );
    }

    #[test]
    fn lone_open_bracket_declined() {
        // ［X］ without ＃ is not our annotation; let comrak treat it as text.
        assert!(adapter().try_parse_inline(ctx("［X］", "")).is_none());
    }

    #[test]
    fn recognises_gaiji_marker() {
        let m = adapter()
            .try_parse_inline(ctx("※［＃「木＋吶」、第3水準1-85-54］後", ""))
            .expect("gaiji");
        assert!(matches!(m.node, AozoraNode::Annotation(_)));
    }

    #[test]
    fn lone_reference_mark_declined() {
        assert!(adapter().try_parse_inline(ctx("※の注", "")).is_none());
    }

    #[test]
    fn unclosed_bracket_declined_for_graceful_degradation() {
        // No ］ to close — leave as text so comrak still round-trips the input.
        assert!(
            adapter()
                .try_parse_inline(ctx("［＃unclosed", ""))
                .is_none()
        );
    }
}
