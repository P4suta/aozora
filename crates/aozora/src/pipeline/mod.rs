#![forbid(unsafe_code)]

mod fold;
pub(crate) mod lexer;
pub(crate) mod state_machine;

use crate::scan;
pub(crate) use crate::syntax::ast::{LexOutput, RegionOutput, SanitizedText, SourceNode};
pub(crate) use fold::{lex, lex_region, lex_shared};

pub(crate) fn prewarm() {
    scan::prewarm();
    lexer::classify::prewarm();
}

pub(crate) use lexer::sanitize::{has_long_rule_line, isolate_decorative_rules};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{
        BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, Diagnostic,
        INLINE_SENTINEL, Sentinel,
    };

    #[test]
    fn lex_produces_normalized_with_pua_sentinels_for_trigger_inputs() {
        let out = lex("｜青梅《おうめ》");
        let inline_count = out
            .normalized
            .chars()
            .filter(|c| *c == INLINE_SENTINEL)
            .count();
        assert_eq!(inline_count, 1, "normalized: {:?}", out.normalized);
        assert_eq!(out.registry.count_kind(Sentinel::Inline), 1);
    }

    #[test]
    fn lex_passes_through_plain_text_unchanged() {
        let out = lex("hello, world");
        assert_eq!(out.normalized, "hello, world");
        assert!(out.registry.is_empty());
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn lex_re_exports_sentinel_constants() {
        assert_eq!(INLINE_SENTINEL, '\u{E001}');
        assert_eq!(BLOCK_LEAF_SENTINEL, '\u{E002}');
        assert_eq!(BLOCK_OPEN_SENTINEL, '\u{E003}');
        assert_eq!(BLOCK_CLOSE_SENTINEL, '\u{E004}');
    }

    #[test]
    fn lex_handles_empty_input() {
        let out = lex("");
        assert!(out.normalized.is_empty());
        assert!(out.registry.is_empty());
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn lex_emits_diagnostics_for_pua_collision() {
        let out = lex("abc\u{E001}def");
        assert!(
            out.diagnostics
                .iter()
                .any(|d| matches!(d, Diagnostic::SourceContainsPua { .. })),
            "expected SourceContainsPua, got {:?}",
            out.diagnostics
        );
    }

    #[test]
    fn lex_preserves_sanitized_len_for_segment_merge() {
        let out = lex("plain text");
        assert_eq!(out.sanitized.len(), "plain text".len());
    }
}
