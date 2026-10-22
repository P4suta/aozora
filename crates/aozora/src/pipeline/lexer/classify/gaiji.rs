use super::super::pair::{PairEvent, PairKind};
use super::super::token::TriggerKind;
use super::{BodyView, RecogniseCtx};
use crate::encoding::gaiji as gaiji_resolve;
use crate::syntax::Span;
use crate::syntax::ast::Gaiji;

pub(super) struct GaijiMatch {
    pub(super) payload: Gaiji,
    pub(super) consume_start: u32,
    pub(super) consume_end: u32,
}

impl RecogniseCtx<'_, '_> {
    pub(super) fn recognize_gaiji(
        &mut self,
        view: BodyView<'_>,
        refmark_span: Span,
        bracket_open_idx: usize,
    ) -> Option<GaijiMatch> {
        self.recognize_gaiji_core(view, refmark_span.start, false, bracket_open_idx)
    }

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

        let body = &self.source[hash_end as usize..bracket_close_span.start as usize];
        let gaiji_resolve::GaijiBody {
            description,
            mencode,
            mencode_separator,
            ..
        } = gaiji_resolve::recognize_gaiji_body(body)?;

        let mut payload = self.alloc.make_gaiji(description, mencode, standalone);
        payload.mencode_separator = mencode_separator;
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
        assert!(is_page_line_shaped("上巻-34-18"));
        assert!(is_page_line_shaped("下巻-68-4"));
        assert!(is_page_line_shaped("7巻-42-下-10"));
        assert!(!is_page_line_shaped(""));
        assert!(!is_page_line_shaped("漢字"));
        assert!(!is_page_line_shaped("U+304B"));
        assert!(!is_page_line_shaped("巻-3-4"));
    }

    #[test]
    fn mencode_token_drops_trailing_page_line() {
        assert_eq!(
            mencode_resolution_token("第3水準1-84-27、144-上-9"),
            "第3水準1-84-27"
        );
        assert_eq!(mencode_resolution_token("U+74FC、372-10"), "U+74FC");
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
