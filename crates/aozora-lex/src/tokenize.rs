//! Trigger-aware tokenisation, driven by [`aozora_scan`].
//!
//! Replaces the legacy [`aozora_lexer::tokenize`] character-by-character
//! walker with an offset-driven loop:
//!
//! 1. [`aozora_scan::TriggerScanner`] returns the byte offsets of every
//!    trigger character in `source` (already SIMD-vectorised through
//!    `memchr3` / AVX2 internally).
//! 2. A single [`memchr::memchr_iter`] sweep collects every newline
//!    offset in one pass — both passes amortise the source's memory-
//!    bandwidth cost.
//! 3. The two sorted offset streams are **merge-walked** in O(T+N)
//!    time, emitting `Text` / `Trigger` / `Newline` tokens in the
//!    same shape the legacy Phase 1 produced.
//! 4. Adjacent `RubyOpen` / `RubyClose` triggers fold into the
//!    `DoubleRubyOpen` / `DoubleRubyClose` two-character variants
//!    via single-step look-ahead on the trigger offset list.
//!
//! ## Why this is byte-identical to legacy Phase 1
//!
//! Both implementations partition `source` into the same three event
//! kinds (`Text`, `Trigger`, `Newline`); the difference is only in
//! how event positions are located. The byte-identical proptest in
//! `tests/property_byte_identical.rs` pins this equivalence on
//! random aozora-shaped input.
//!
//! ## Why merge-walk over per-trigger memchr
//!
//! The first version of this module called `memchr::memchr_iter` for
//! newlines INSIDE the trigger walk loop, scanning one segment of
//! source bytes per trigger. That doubled the source memory-bandwidth
//! cost (one pass for triggers via `aozora-scan`, a second pass for
//! newlines amortised across segments) and measured 35% slower on
//! the corpus profile than the legacy char-by-char tokenizer despite
//! the SIMD scan. Pre-collecting newlines via a single bulk
//! `memchr_iter` and then merge-walking the two sorted streams keeps
//! source memory traffic at the same level as legacy Phase 1.

use aozora_lexer::Token;
use aozora_spec::{Span, TriggerKind, classify_trigger_bytes};

/// Tokenise `source` into the same `Vec<Token>` shape the legacy
/// Phase 1 produced.
///
/// Uses [`aozora_scan::best_scanner`] for the trigger discovery
/// sweep and a single [`memchr::memchr_iter`] for newline discovery.
///
/// # Panics
///
/// Panics if `source.len()` exceeds `u32::MAX`. All `Span` offsets in
/// the workspace are `u32`, so this bound is the upper limit of the
/// representable source size (~4 GiB).
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    reason = "function entry asserts source.len() <= u32::MAX, so every byte index fits"
)]
pub fn tokenize_with_scan(source: &str) -> Vec<Token> {
    assert!(
        u32::try_from(source.len()).is_ok(),
        "source too long for u32 span offsets ({} bytes)",
        source.len()
    );

    let scanner = aozora_scan::best_scanner();
    let trigger_offsets = scanner.scan_offsets(source);
    let bytes = source.as_bytes();

    // Bulk newline collection — one pass through source, vectorised
    // by `memchr` internally.
    let mut newline_offsets: Vec<u32> = Vec::with_capacity(bytes.len() / 64);
    for nl in memchr::memchr_iter(b'\n', bytes) {
        // Cast safe: source.len() <= u32::MAX (asserted above), so
        // every byte index fits.
        #[allow(
            clippy::cast_possible_truncation,
            reason = "source.len() <= u32::MAX is asserted at function entry"
        )]
        newline_offsets.push(nl as u32);
    }

    // Capacity heuristic: legacy Phase 1 uses `source.len() / 32`.
    // We match it so `Vec` growth costs are equivalent.
    let mut tokens: Vec<Token> = Vec::with_capacity(bytes.len() / 32);

    let mut text_start: u32 = 0;
    let mut t_idx: usize = 0;
    let mut n_idx: usize = 0;

    // Merge-walk the two sorted streams in event order.
    while t_idx < trigger_offsets.len() || n_idx < newline_offsets.len() {
        let next_is_trigger = match (
            trigger_offsets.get(t_idx),
            newline_offsets.get(n_idx),
        ) {
            (Some(&t), Some(&n)) => t < n,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => unreachable!("loop guard"),
        };

        if next_is_trigger {
            let trigger_pos: usize = trigger_offsets[t_idx] as usize;

            // Emit pending text up to the trigger.
            if (trigger_pos as u32) > text_start {
                push_text(&mut tokens, text_start, trigger_pos as u32);
            }

            // Classify the trigger window. The scanner's contract
            // guarantees this returns Some.
            let kind = trigger_kind_at(bytes, trigger_pos);

            // Look-ahead for the double-trigger merge.
            let (emit_kind, byte_len, consumed_extra) =
                try_merge_double_trigger(bytes, &trigger_offsets, t_idx, kind);

            let span = Span::new(trigger_pos as u32, trigger_pos as u32 + byte_len);
            tokens.push(Token::Trigger { kind: emit_kind, span });

            let after_trigger = trigger_pos as u32 + byte_len;
            text_start = after_trigger;
            t_idx += 1 + consumed_extra;

            // Skip any newline offsets that the merged trigger now
            // covers (only possible for the merged double variant
            // where we consumed an extra trigger; newlines never
            // overlap a trigger because the scanner doesn't emit
            // them).
            while n_idx < newline_offsets.len() && newline_offsets[n_idx] < after_trigger {
                n_idx += 1;
            }
        } else {
            let nl_pos: u32 = newline_offsets[n_idx];
            // Emit pending text up to the newline.
            if nl_pos > text_start {
                push_text(&mut tokens, text_start, nl_pos);
            }
            tokens.push(Token::Newline { pos: nl_pos });
            text_start = nl_pos + 1;
            n_idx += 1;
        }
    }

    // Tail: any text after the last event.
    let total_len = bytes.len() as u32;
    if total_len > text_start {
        push_text(&mut tokens, text_start, total_len);
    }

    tokens
}

/// Look at the 3-byte window at `pos` and return its [`TriggerKind`].
/// Caller guarantees `pos + 3 <= bytes.len()` and that the window is
/// in fact a recognised trigger (the scanner's contract).
#[inline]
fn trigger_kind_at(bytes: &[u8], pos: usize) -> TriggerKind {
    let window: [u8; 3] = [bytes[pos], bytes[pos + 1], bytes[pos + 2]];
    classify_trigger_bytes(window).expect("scanner only emits classified positions")
}

/// Single-step look-ahead for the `《《` / `》》` double-trigger
/// merge. Returns `(kind_to_emit, byte_len_in_source, extra_offsets_to_skip)`.
///
/// Mirrors the legacy [`aozora_lexer::tokenize`]'s merge contract:
/// when a `RubyOpen` / `RubyClose` is *immediately* followed (no gap
/// in source bytes) by another of the same kind, fold them into the
/// double variant covering 6 source bytes.
#[inline]
fn try_merge_double_trigger(
    bytes: &[u8],
    trigger_offsets: &[u32],
    t_idx: usize,
    kind: TriggerKind,
) -> (TriggerKind, u32, usize) {
    let trigger_pos = trigger_offsets[t_idx] as usize;
    let needed_double = match kind {
        TriggerKind::RubyOpen => Some(TriggerKind::DoubleRubyOpen),
        TriggerKind::RubyClose => Some(TriggerKind::DoubleRubyClose),
        _ => None,
    };
    let Some(merged_kind) = needed_double else {
        return (kind, 3, 0);
    };

    // Next offset, if any, must sit exactly 3 bytes after the current
    // one (i.e., the very next character in source) AND classify as
    // the same single-character kind.
    let next_idx = t_idx + 1;
    if next_idx >= trigger_offsets.len() {
        return (kind, 3, 0);
    }
    let next_pos = trigger_offsets[next_idx] as usize;
    if next_pos != trigger_pos + 3 {
        return (kind, 3, 0);
    }
    let next_kind = trigger_kind_at(bytes, next_pos);
    if next_kind != kind {
        return (kind, 3, 0);
    }
    (merged_kind, 6, 1)
}

/// Push a non-empty `Text` token covering `[start, end)`.
#[inline]
fn push_text(tokens: &mut Vec<Token>, start: u32, end: u32) {
    if end > start {
        tokens.push(Token::Text {
            range: Span::new(start, end),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Equivalence baseline: for any input, our tokenizer produces
    /// the same token stream the legacy phase 1 would.
    #[track_caller]
    fn assert_tokens_match_legacy(source: &str) {
        let ours = tokenize_with_scan(source);
        let theirs = aozora_lexer::tokenize(source);
        assert_eq!(
            ours, theirs,
            "token streams diverged for input {source:?}\n  ours:   {ours:?}\n  legacy: {theirs:?}"
        );
    }

    #[test]
    fn empty_input_produces_no_tokens() {
        assert_tokens_match_legacy("");
    }

    #[test]
    fn plain_ascii_collapses_to_a_single_text_token() {
        assert_tokens_match_legacy("Hello, world.");
    }

    #[test]
    fn plain_japanese_collapses_to_a_single_text_token() {
        assert_tokens_match_legacy("こんにちは、世界！");
    }

    #[test]
    fn newline_alone_emits_a_newline_token() {
        assert_tokens_match_legacy("\n");
    }

    #[test]
    fn text_with_trailing_newline_emits_text_then_newline() {
        assert_tokens_match_legacy("hello\n");
    }

    #[test]
    fn multiple_newlines_are_each_their_own_token() {
        assert_tokens_match_legacy("a\nb\n\nc");
    }

    #[test]
    fn explicit_ruby_emits_text_bar_text_open_text_close() {
        assert_tokens_match_legacy("｜青梅《おうめ》");
    }

    #[test]
    fn implicit_ruby_emits_text_open_text_close() {
        assert_tokens_match_legacy("青梅《おうめ》");
    }

    #[test]
    fn double_ruby_open_close_merge_into_single_tokens() {
        assert_tokens_match_legacy("《《重要》》");
    }

    #[test]
    fn bracket_annotation_emits_open_hash_text_close() {
        assert_tokens_match_legacy("text［＃改ページ］more");
    }

    #[test]
    fn nested_quote_inside_annotation_round_trips() {
        assert_tokens_match_legacy("［＃「青空」に傍点］");
    }

    #[test]
    fn gaiji_marker_sequence() {
        assert_tokens_match_legacy("※［＃「木＋吶のつくり」、第3水準1-85-54］");
    }

    #[test]
    fn tortoise_brackets_mark_accent_segment() {
        assert_tokens_match_legacy("〔fune`bre〕");
    }

    #[test]
    fn three_consecutive_ruby_opens_merge_first_two() {
        // 《《《X》 — the first two merge to DoubleRubyOpen, the
        // third stays as a single RubyOpen. This matches the legacy
        // greedy left-to-right merge.
        assert_tokens_match_legacy("《《《X》");
    }

    #[test]
    fn isolated_single_close_does_not_merge_into_double() {
        assert_tokens_match_legacy("text》more");
    }

    #[test]
    fn long_run_with_mixed_constructs() {
        assert_tokens_match_legacy(
            "明治の頃｜青梅《おうめ》街道沿いに、※［＃「木＋吶のつくり」、第3水準1-85-54］\n\
             なる珍しき木が立つ。［＃ここから2字下げ］その下で人々は語らひ。\n\
             ［＃ここで字下げ終わり］",
        );
    }

    #[test]
    fn newlines_inside_a_ruby_span_are_their_own_tokens() {
        assert_tokens_match_legacy("｜abc\n《def》");
    }

    #[test]
    fn pua_collision_in_source_is_passed_through_as_text() {
        assert_tokens_match_legacy("abc\u{E001}def");
    }
}
