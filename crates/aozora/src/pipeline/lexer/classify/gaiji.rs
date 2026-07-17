//! Gaiji (外字) reference recognition.
//!
//! The `※［＃<description>、<mencode>］` glyph-reference form plus the
//! `mencode` / 底本ページ-行 validators. Extracted verbatim from the
//! classify-stage classifier; recognised nodes are built on the shared
//! [`RecogniseCtx`].

#[cfg(feature = "classify-instrument")]
use super::super::instrumentation::{Subsystem, SubsystemGuard};
use super::super::pair::{PairEvent, PairKind};
use super::super::token::TriggerKind;
use super::{BodyView, RecogniseCtx};
use crate::encoding::gaiji as gaiji_resolve;
use crate::syntax::Span;
use crate::syntax::ast::Gaiji;

/// Intermediate result of `recognize_gaiji`.
///
/// Holds the payload (`Gaiji`) rather than a wrapped
/// node so the caller can route it to either `alloc.gaiji(p)`
/// (top-level span) or `alloc.seg_gaiji(p)` (nested inside a body
/// content) without re-paying the description / mencode intern cost.
pub(super) struct GaijiMatch {
    pub(super) payload: Gaiji,
    pub(super) consume_start: u32,
    pub(super) consume_end: u32,
}

/// Try to recognize a gaiji reference at `events[refmark_idx]`.
///
/// Shape: `※［＃<description>、<mencode>］` or `※［＃<description>］`.
/// The description may be wrapped in `「…」` (the common form) or
/// appear bare. `<mencode>` is the mencode reference (`第3水準1-85-54`,
/// `U+XXXX`, etc.) appearing after a `、` separator.
///
/// The mencode tail is classified into its `GaijiCanonical` form by
/// `make_gaiji`; the resolved glyph is derived on demand via
/// `Gaiji::resolve` (which calls the same `crate::encoding::gaiji`
/// lookup authority), so the recogniser stores no eager `ucs` column.
///
/// Event preconditions (checked):
/// * `events[refmark_idx]` is `Solo(RefMark)` [done by caller]
/// * `events[refmark_idx + 1]` is `PairOpen(Bracket)` [done by caller]
/// * `events[refmark_idx + 2]` is `Solo(Hash)` [checked here]
///
/// Consume range is from `refmark_span.start` to the bracket close's
/// end — i.e. the `※` and the entire following `［＃…］` fold into
/// one Aozora span.
impl RecogniseCtx<'_, '_> {
    pub(super) fn recognize_gaiji(
        &mut self,
        view: BodyView<'_>,
        refmark_span: Span,
        bracket_open_idx: usize,
    ) -> Option<GaijiMatch> {
        self.recognize_gaiji_core(view, refmark_span.start, false, bracket_open_idx)
    }

    /// Core recogniser shared by the `※`-prefixed [`Self::recognize_gaiji`]
    /// and the no-refmark standalone (#122) form. `consume_start` is the
    /// byte the gaiji span folds back to (`※` start for the refmark form,
    /// `［` start for standalone); `standalone` records whether the source
    /// carried a `※` so the serializer can round-trip the bracket verbatim.
    #[expect(
        clippy::too_many_arguments,
        reason = "a flat gaiji body recogniser over four independent inputs (view, \
                  consume_start, standalone, bracket_open_idx); splitting it would \
                  scatter the I3-idempotency reasoning"
    )]
    pub(super) fn recognize_gaiji_core(
        &mut self,
        view: BodyView<'_>,
        consume_start: u32,
        standalone: bool,
        bracket_open_idx: usize,
    ) -> Option<GaijiMatch> {
        #[cfg(feature = "classify-instrument")]
        let _classify_guard = SubsystemGuard::new(Subsystem::Gaiji);
        let events = view.events;
        let &PairEvent::PairOpen {
            kind: PairKind::Bracket,
            ..
        } = events.get(bracket_open_idx)?
        else {
            return None;
        };
        let bracket_close_link = *view.links.get(bracket_open_idx)?;
        if bracket_close_link == u32::MAX {
            return None;
        }
        let bracket_close_idx = bracket_close_link as usize;
        let hash_end = match events.get(bracket_open_idx + 1)? {
            PairEvent::Solo {
                kind: TriggerKind::Hash,
                span,
            } => span.end,
            _ => return None,
        };
        let &PairEvent::PairClose {
            span: bracket_close_span,
            ..
        } = events.get(bracket_close_idx)?
        else {
            return None;
        };

        // Split the body into (description, mencode) via the single
        // authority in the encoding layer — shared with the LSP resolution
        // view and the gaiji() wire. It handles the simple quoted form, the
        // composed-glyph / 正字 / 屋号 forms, and the bare form with one
        // right-to-left mencode scan (the naive first-`、` split it replaces
        // wrongly cut composed forms — #181).
        // Recognition gate (I3 idempotency) via the single authority in
        // the encoding layer — shared with the resolution view (#181) and the
        // gaiji() wire. The simple `「desc」` quoted form is a gaiji even
        // without a mencode; the composed / bare forms need a trailing mencode
        // anchor; a non-serializable description (stray quote imbalance, an
        // embedded `［＃`) is declined. Declined brackets fall through to
        // `Directive{Unknown}` and round-trip byte-identical.
        let body = &self.source[hash_end as usize..bracket_close_span.start as usize];
        let gaiji_resolve::GaijiBody {
            description,
            mencode,
            ..
        } = gaiji_resolve::recognize_gaiji_body(body)?;

        // The mencode is kept verbatim (a trailing 底本ページ-行 suffix and
        // all) inside the GaijiCanonical; resolution to a glyph happens
        // lazily via `Gaiji::resolve`, which strips the suffix for the
        // men-ku-ten lookup. No eager `ucs` column is stored.
        let payload = self.alloc.make_gaiji(description, mencode, standalone);
        Some(GaijiMatch {
            payload,
            consume_start,
            consume_end: bracket_close_span.end,
        })
    }
}

#[cfg(test)]
mod is_mencode_shaped_tests {
    use crate::encoding::gaiji::{
        is_mencode_shaped, is_page_line_shaped, mencode_resolution_token,
    };

    #[test]
    fn page_line_shaped_accepts_corpus_forms() {
        assert!(is_page_line_shaped("372-10"));
        assert!(is_page_line_shaped("144-上-9"));
        assert!(is_page_line_shaped("1-13-25"));
        assert!(is_page_line_shaped("１４４-下-９"));
        // Multi-volume 底本 carry a 巻 marker before the page/line.
        assert!(is_page_line_shaped("上巻-34-18"));
        assert!(is_page_line_shaped("下巻-68-4"));
        assert!(is_page_line_shaped("7巻-42-下-10"));
        assert!(!is_page_line_shaped(""));
        assert!(!is_page_line_shaped("漢字"));
        assert!(!is_page_line_shaped("U+304B"));
        // `巻` alone (no column / number prefix) is not a page-line part.
        assert!(!is_page_line_shaped("巻-3-4"));
    }

    #[test]
    fn mencode_token_drops_trailing_page_line() {
        assert_eq!(
            mencode_resolution_token("第3水準1-84-27、144-上-9"),
            "第3水準1-84-27"
        );
        assert_eq!(mencode_resolution_token("U+74FC、372-10"), "U+74FC");
        // No page-line suffix — returned unchanged.
        assert_eq!(mencode_resolution_token("第3水準1-85-54"), "第3水準1-85-54");
    }

    #[test]
    fn jis_x_0213_row_cell_passes() {
        assert!(is_mencode_shaped("1-2-23"));
        assert!(is_mencode_shaped("1-85-54"));
    }

    #[test]
    fn dai_n_suijun_prefix_passes() {
        assert!(is_mencode_shaped("第3水準1-85-54"));
        assert!(is_mencode_shaped("第4水準2-1-2"));
    }

    #[test]
    fn u_plus_codepoint_passes() {
        assert!(is_mencode_shaped("U+304B"));
        assert!(is_mencode_shaped("U+1F94A"));
    }

    #[test]
    fn random_garbage_rejected() {
        assert!(!is_mencode_shaped("》"));
        assert!(!is_mencode_shaped("abc"));
        assert!(!is_mencode_shaped("漢字"));
        assert!(!is_mencode_shaped(""));
        assert!(!is_mencode_shaped("U+"));
        assert!(!is_mencode_shaped("U+ZZZZ"));
        assert!(!is_mencode_shaped("第3水準"));
    }
}
