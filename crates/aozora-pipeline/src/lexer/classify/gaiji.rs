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
use aozora_encoding::gaiji as gaiji_resolve;
use aozora_syntax::{Span, borrowed};

/// Intermediate result of `recognize_gaiji`.
///
/// Holds the payload (`&'a borrowed::Gaiji<'a>`) rather than a wrapped
/// node so the caller can route it to either `alloc.gaiji(p)`
/// (top-level span) or `alloc.seg_gaiji(p)` (nested inside a body
/// content) without re-paying the description / mencode intern cost.
pub(super) struct GaijiMatch<'a> {
    pub(super) payload: &'a borrowed::Gaiji<'a>,
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
/// The UCS resolution column of `Gaiji` is populated by
/// `aozora_encoding::gaiji::lookup` before the recogniser returns, so
/// downstream consumers receive a resolved `Option<char>` without
/// having to re-probe the mencode table.
///
/// Event preconditions (checked):
/// * `events[refmark_idx]` is `Solo(RefMark)` [done by caller]
/// * `events[refmark_idx + 1]` is `PairOpen(Bracket)` [done by caller]
/// * `events[refmark_idx + 2]` is `Solo(Hash)` [checked here]
///
/// Consume range is from `refmark_span.start` to the bracket close's
/// end — i.e. the `※` and the entire following `［＃…］` fold into
/// one Aozora span.
impl<'a> RecogniseCtx<'_, 'a, '_> {
    pub(super) fn recognize_gaiji(
        &mut self,
        view: BodyView<'_>,
        refmark_span: Span,
        bracket_open_idx: usize,
    ) -> Option<GaijiMatch<'a>> {
        self.recognize_gaiji_core(view, refmark_span.start, false, bracket_open_idx)
    }

    /// Core recogniser shared by the `※`-prefixed [`Self::recognize_gaiji`]
    /// and the no-refmark standalone (#122) form. `consume_start` is the
    /// byte the gaiji span folds back to (`※` start for the refmark form,
    /// `［` start for standalone); `standalone` records whether the source
    /// carried a `※` so the serializer can round-trip the bracket verbatim.
    #[allow(
        clippy::too_many_arguments,
        clippy::too_many_lines,
        reason = "a flat gaiji body recogniser (quoted form, bare form, validation, \
                  resolution) over four independent inputs; splitting it would scatter \
                  the I3-idempotency reasoning"
    )]
    pub(super) fn recognize_gaiji_core(
        &mut self,
        view: BodyView<'_>,
        consume_start: u32,
        standalone: bool,
        bracket_open_idx: usize,
    ) -> Option<GaijiMatch<'a>> {
        #[cfg(feature = "classify-instrument")]
        let _phase3_guard = SubsystemGuard::new(Subsystem::Gaiji);
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
        // authority in aozora-encoding — shared with the LSP resolution
        // view and the gaiji() wire. It handles the simple quoted form, the
        // composed-glyph / 正字 / 屋号 forms, and the bare form with one
        // right-to-left mencode scan (the naive first-`、` split it replaces
        // wrongly cut composed forms — #181).
        let body = &self.source[hash_end as usize..bracket_close_span.start as usize];
        let gaiji_resolve::GaijiBody {
            description,
            mencode,
            quoted,
        } = gaiji_resolve::parse_gaiji_body(body);

        // Recognition gate (I3 idempotency): the simple `「desc」` quoted form
        // is a gaiji even without a mencode; the composed / bare forms need a
        // trailing mencode anchor. A non-serializable description (stray quote
        // imbalance, an embedded `［＃`) is also declined. Declined brackets
        // fall through to `Directive{Unknown}` and round-trip byte-identical.
        if description.is_empty()
            || (!quoted && mencode.is_none())
            || !gaiji_description_serializable(description, mencode.is_some())
        {
            return None;
        }

        // Resolve the Unicode scalar at lex time via the static table in
        // aozora-encoding so the downstream AST / renderer never has to
        // re-probe. A trailing 底本ページ-行 suffix is stripped for resolution
        // (the men-ku-ten still maps the glyph) while the full mencode is
        // stored verbatim for round-trip.
        let ucs = gaiji_resolve::lookup(
            None,
            mencode.map(gaiji_resolve::mencode_resolution_token),
            description,
        );

        let payload = self.alloc.make_gaiji(description, ucs, mencode, standalone);
        Some(GaijiMatch {
            payload,
            consume_start,
            consume_end: bracket_close_span.end,
        })
    }
}

/// Whether a gaiji `description` can be kept (it both serializes and
/// round-trips); otherwise the bracket falls through to `Directive{Unknown}`.
///
/// Rejects:
///   - a description embedding `［＃` (a nested annotation opener would leak a
///     bare `［＃` outside the `aozora-directive` wrapper, violating the Tier A
///     canary), and
///   - a description carrying structural `「…」` quotes, *except* the
///     composed-glyph / 正字 / 屋号 forms (`「X」の「Y」に代えて「Z」`,
///     `「…）、「柿」の正字」`, `「…」、屋号を示す記号`; corpus §6 external-character
///     forms): balanced quotes anchored by a trailing `、mencode`. `emit_gaiji`
///     writes such a description verbatim (no `「…」` wrapper), so it round-trips
///     even when it carries an internal `、` — the recogniser's right-to-left
///     mencode scan (`parse_gaiji_body`, the aozora-encoding authority)
///     re-splits at the same boundary. A quote-bearing description without a
///     mencode anchor stays
///     rejected (the serializer's wrapper would unbalance it).
fn gaiji_description_serializable(description: &str, has_mencode: bool) -> bool {
    if description.contains("［＃") {
        return false;
    }
    if description.contains(['「', '」']) {
        let balanced = description.matches('「').count() == description.matches('」').count();
        return balanced && has_mencode;
    }
    true
}

#[cfg(test)]
mod is_mencode_shaped_tests {
    use aozora_encoding::gaiji::{
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
