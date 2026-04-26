//! Phase 1 — linear tokenization of sanitized source into a token stream.
//!
//! Walks the Phase 0 sanitized text via the SIMD-accelerated
//! [`aozora_scan`] crate and exposes a stateful iterator yielding one
//! [`Token`] per delimiter or contiguous text run. Triggers are the
//! Aozora notation marker characters listed in [`TriggerKind`];
//! everything else flows into [`Token::Text`] runs.
//!
//! Two production-ready surfaces sit side by side:
//!
//! - [`tokenize`] — streaming `impl Iterator<Item = Token>`, kept for
//!   FFI / incremental / pull-based consumers that have no arena.
//! - [`tokenize_in`] — arena-batch `BumpVec<'a, Token>` allocated
//!   inside the caller's [`Arena`]. Used by the borrowed pipeline:
//!   one bump-pointer advance per token replaces N heap mallocs per
//!   parse (R4-A / ADR-0017).
//!
//! The Aozora pipeline drives `tokenize_in` because it already owns an
//! arena; benchmarks and FFI shims that want lazy semantics use
//! `tokenize`. There is no third "heap-batch" entry point — R2 added
//! one (`tokenize_to_vec`); R4-A removed it once the arena migration
//! made it dead code.
//!
//! ## Algorithm (post-T2 / ADR-0015)
//!
//! 1. [`aozora_scan::best_scanner`] returns the byte offsets of every
//!    trigger character in `source`. On `x86_64` this dispatches to
//!    Teddy (Hyperscan multi-pattern fingerprint matcher); on minimal
//!    hosts to a SIMD-free DFA. Both produce byte-identical output.
//! 2. A single [`memchr::memchr_iter`] sweep collects every newline
//!    offset. Together with step 1, source bytes are touched twice
//!    (once per scan), both at near memory-bandwidth speed.
//! 3. [`Iterator::next`] merge-walks the two sorted offset streams
//!    in event order, emitting `Text` / `Trigger` / `Newline` tokens.
//! 4. Adjacent `RubyOpen` / `RubyClose` triggers fold into the
//!    `DoubleRubyOpen` / `DoubleRubyClose` two-character variants
//!    via single-step look-ahead on the trigger offset list.
//!
//! `［＃` is NOT emitted as a merged trigger: `Hash` after
//! `BracketOpen` is common but not universal (a stray `［` followed
//! by plain text is legal). Phase 2 inspects the two tokens together.
//!
//! ## History
//!
//! - **T1 (2026-04, reverted)**: first SIMD attempt used the
//!   leading-byte filter `{0xE2, 0xE3, 0xEF}`. ADR-0013 records the
//!   3.7× regression on Japanese caused by `0xE3` saturating the
//!   candidate stream.
//! - **T2 (2026-04, this revision)**: ADR-0015 documents the
//!   four-backend bake-off that picked Teddy. Bake-off measured
//!   19.4 GiB/s on plain Japanese, 10.8 GiB/s at corpus-median
//!   trigger density, vs the legacy walker's ~150 MiB/s.

use aozora_spec::classify_trigger_bytes;
use aozora_syntax::Span;
use aozora_syntax::borrowed::Arena;
use bumpalo::collections::Vec as BumpVec;

use crate::token::{Token, TriggerKind};

/// Streaming tokeniser over sanitized source text.
///
/// The input is expected to already be Phase 0 output (BOM-stripped,
/// LF-normalized). Giving raw source to this iterator is not wrong but
/// means diagnostics and positions reference pre-normalization bytes,
/// which will confuse downstream phases.
///
/// # Panics
///
/// Panics on construction if `source.len()` exceeds [`u32::MAX`]
/// (≈ 4 GiB). All afm spans use `u32` offsets per the
/// `aozora-syntax::Span` contract; inputs that large are rejected
/// loudly rather than silently truncated.
#[must_use]
pub fn tokenize(source: &str) -> Tokenizer<'_> {
    Tokenizer::new(source)
}

/// Materialise every Phase 1 token into an arena-backed
/// [`bumpalo::collections::Vec`] in one pass.
///
/// R4-A (ADR-0017): the inter-phase token list is allocated inside the
/// caller's arena instead of on the heap. The borrowed pipeline already
/// owns one [`Arena`] per parse — collapsing per-parse `Vec<Token>`
/// `malloc`/`free` traffic into a single bump-pointer advance per token
/// removed allocation from the corpus profile's top bucket. The
/// streaming [`tokenize`] iterator is kept for incremental / FFI
/// consumers that pull lazily and have no arena to spend.
///
/// Internally this is exactly the merge-walk [`Tokenizer::next`] runs,
/// flattened into a single `for` loop pushing into a pre-sized `BumpVec`.
/// Drops the `pending: Option<Token>` slot (the streaming buffer for
/// "Text+Trigger emitted together") because direct pushes can write
/// both events back-to-back.
///
/// Capacity hint: 2 tokens per trigger (text+trigger) + one per
/// newline + a small fixed overhead. Slight over-estimate is cheap
/// and avoids the `BumpVec` re-grow path (which moves the previous
/// allocation forward in the arena, doubling its footprint).
///
/// # Panics
///
/// Same as [`tokenize`]: panics if `source.len()` exceeds [`u32::MAX`].
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    reason = "function entry asserts source.len() <= u32::MAX, so every byte index fits"
)]
pub fn tokenize_in<'a>(source: &str, arena: &'a Arena) -> BumpVec<'a, Token> {
    assert!(
        u32::try_from(source.len()).is_ok(),
        "source too long for u32 span offsets ({} bytes)",
        source.len()
    );
    let bytes = source.as_bytes();
    let trigger_offsets = aozora_scan::best_scanner().scan_offsets(source);
    let mut newline_offsets: Vec<u32> = Vec::with_capacity(bytes.len() / 64);
    for n in memchr::memchr_iter(b'\n', bytes) {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "source.len() <= u32::MAX is asserted at function entry"
        )]
        newline_offsets.push(n as u32);
    }

    let cap = trigger_offsets.len() * 2 + newline_offsets.len() + 8;
    let mut out: BumpVec<'a, Token> = BumpVec::with_capacity_in(cap, arena.bump());

    let mut text_start: u32 = 0;
    let mut t_idx: usize = 0;
    let mut n_idx: usize = 0;

    loop {
        let t_offset = trigger_offsets.get(t_idx).copied();
        let n_offset = newline_offsets.get(n_idx).copied();
        let next_is_trigger = match (t_offset, n_offset) {
            (Some(t), Some(n)) => t < n,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => break,
        };

        if next_is_trigger {
            let t_pos = t_offset.expect("checked Some by next_is_trigger arm");
            let kind = trigger_kind_at(bytes, t_pos as usize);
            let (emit_kind, byte_len, extra) =
                merge_double(bytes, &trigger_offsets, t_idx, kind, t_pos);
            if t_pos > text_start {
                out.push(Token::Text {
                    range: Span::new(text_start, t_pos),
                });
            }
            out.push(Token::Trigger {
                kind: emit_kind,
                span: Span::new(t_pos, t_pos + byte_len),
            });
            let after = t_pos + byte_len;
            text_start = after;
            t_idx += 1 + extra;
            while n_idx < newline_offsets.len() && newline_offsets[n_idx] < after {
                n_idx += 1;
            }
        } else {
            let n_pos = n_offset.expect("checked Some by !next_is_trigger arm");
            if n_pos > text_start {
                out.push(Token::Text {
                    range: Span::new(text_start, n_pos),
                });
            }
            out.push(Token::Newline { pos: n_pos });
            text_start = n_pos + 1;
            n_idx += 1;
        }
    }

    let total_len = bytes.len() as u32;
    if total_len > text_start {
        out.push(Token::Text {
            range: Span::new(text_start, total_len),
        });
    }
    out
}

/// Free-function variant of [`Tokenizer::try_merge_double`] used by
/// [`tokenize_in`]. Returns `(kind, byte_len, extra_idx)`.
#[inline]
#[allow(
    clippy::too_many_arguments,
    reason = "five small u32/usize/slice/byte-slice args; bundling into a struct would obscure the inner-loop hot path"
)]
fn merge_double(
    bytes: &[u8],
    trigger_offsets: &[u32],
    t_idx: usize,
    kind: TriggerKind,
    t_pos: u32,
) -> (TriggerKind, u32, usize) {
    let merged = match kind {
        TriggerKind::RubyOpen => TriggerKind::DoubleRubyOpen,
        TriggerKind::RubyClose => TriggerKind::DoubleRubyClose,
        _ => return (kind, 3, 0),
    };
    let next_idx = t_idx + 1;
    let Some(&next_pos) = trigger_offsets.get(next_idx) else {
        return (kind, 3, 0);
    };
    if next_pos != t_pos + 3 {
        return (kind, 3, 0);
    }
    let next_kind = trigger_kind_at(bytes, next_pos as usize);
    if next_kind != kind {
        return (kind, 3, 0);
    }
    (merged, 6, 1)
}

/// Streaming Phase 1 tokeniser over the merge of two pre-collected
/// offset streams: trigger positions (from the SIMD scanner) and
/// newline positions (from `memchr`).
///
/// The single-slot `pending` buffer holds a Trigger / Newline that
/// was produced *together* with a closing Text run on the same
/// `next()` call: [`Iterator::next`] returns the Text first, then
/// the buffered event on the following call, preserving the legacy
/// emission order without paying for a `Vec` accumulator.
#[derive(Debug)]
pub struct Tokenizer<'s> {
    source: &'s str,
    /// Sorted ascending byte offsets where a trigger trigram begins.
    /// Materialised eagerly because the SIMD scanner is much faster
    /// than amortising its internal state across `next()` calls.
    trigger_offsets: Vec<u32>,
    /// Sorted ascending byte offsets of `\n` characters.
    newline_offsets: Vec<u32>,
    t_idx: usize,
    n_idx: usize,
    text_start: u32,
    pending: Option<Token>,
    finished: bool,
}

impl<'s> Tokenizer<'s> {
    fn new(source: &'s str) -> Self {
        assert!(
            u32::try_from(source.len()).is_ok(),
            "source too long for u32 span offsets ({} bytes)",
            source.len()
        );
        let trigger_offsets = aozora_scan::best_scanner().scan_offsets(source);
        let bytes = source.as_bytes();
        // memchr_iter is internally vectorised (AVX2 on x86_64, NEON on
        // aarch64) — the same machine code memchr3 uses for trigger
        // candidates, here narrowed to the single newline byte.
        let mut newline_offsets: Vec<u32> = Vec::with_capacity(bytes.len() / 64);
        for n in memchr::memchr_iter(b'\n', bytes) {
            #[allow(
                clippy::cast_possible_truncation,
                reason = "source.len() <= u32::MAX is asserted at function entry"
            )]
            newline_offsets.push(n as u32);
        }
        Self {
            source,
            trigger_offsets,
            newline_offsets,
            t_idx: 0,
            n_idx: 0,
            text_start: 0,
            pending: None,
            finished: false,
        }
    }

    fn flush_text(&mut self, end: u32) -> Option<Token> {
        (end > self.text_start).then(|| {
            let tok = Token::Text {
                range: Span::new(self.text_start, end),
            };
            self.text_start = end;
            tok
        })
    }

    /// Pair a flushed Text token (if any) with the structural event
    /// that produced the flush. The Text comes first, the event is
    /// buffered for the next `next()` call — preserving emission
    /// order without intermediate allocation.
    fn pair_text_then(&mut self, text: Option<Token>, event: Token) -> Token {
        match text {
            Some(t) => {
                self.pending = Some(event);
                t
            }
            None => event,
        }
    }

    /// Single-step look-ahead for the `《《` / `》》` double-trigger
    /// merge. Returns `(kind_to_emit, byte_len_in_source, extra_offsets_to_consume)`.
    ///
    /// Mirrors the legacy tokenizer's merge contract: when a
    /// `RubyOpen` / `RubyClose` is *immediately* followed (no gap in
    /// source bytes) by another of the same kind, fold them into the
    /// double variant covering 6 source bytes.
    fn try_merge_double(
        &self,
        bytes: &[u8],
        t_pos: u32,
        kind: TriggerKind,
    ) -> (TriggerKind, u32, usize) {
        let merged_kind = match kind {
            TriggerKind::RubyOpen => TriggerKind::DoubleRubyOpen,
            TriggerKind::RubyClose => TriggerKind::DoubleRubyClose,
            _ => return (kind, 3, 0),
        };
        let next_idx = self.t_idx + 1;
        let Some(&next_pos) = self.trigger_offsets.get(next_idx) else {
            return (kind, 3, 0);
        };
        if next_pos != t_pos + 3 {
            return (kind, 3, 0);
        }
        let next_kind = trigger_kind_at(bytes, next_pos as usize);
        if next_kind != kind {
            return (kind, 3, 0);
        }
        (merged_kind, 6, 1)
    }
}

impl Iterator for Tokenizer<'_> {
    type Item = Token;

    fn next(&mut self) -> Option<Token> {
        if let Some(tok) = self.pending.take() {
            return Some(tok);
        }
        if self.finished {
            return None;
        }

        let bytes = self.source.as_bytes();
        let t_offset = self.trigger_offsets.get(self.t_idx).copied();
        let n_offset = self.newline_offsets.get(self.n_idx).copied();

        let next_is_trigger = match (t_offset, n_offset) {
            (Some(t), Some(n)) => t < n,
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => {
                // No more events: emit any trailing text once, then EOF.
                self.finished = true;
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "source.len() <= u32::MAX is asserted at construction"
                )]
                let total_len = bytes.len() as u32;
                return self.flush_text(total_len);
            }
        };

        if next_is_trigger {
            let t_pos = t_offset.expect("checked Some by next_is_trigger arm");
            let kind = trigger_kind_at(bytes, t_pos as usize);
            let (emit_kind, byte_len, extra) = self.try_merge_double(bytes, t_pos, kind);
            let trigger = Token::Trigger {
                kind: emit_kind,
                span: Span::new(t_pos, t_pos + byte_len),
            };
            let text = self.flush_text(t_pos);
            let after = t_pos + byte_len;
            self.text_start = after;
            self.t_idx += 1 + extra;
            // The merged double-trigger may cover newline offsets
            // (in pathological inputs only — `\n` cannot appear inside
            // `《《`/`》》` since both are 6 ASCII-foreign bytes — but
            // we keep the skip loop for defensive symmetry with the
            // tokenize_with_scan reference implementation).
            while self.n_idx < self.newline_offsets.len()
                && self.newline_offsets[self.n_idx] < after
            {
                self.n_idx += 1;
            }
            Some(self.pair_text_then(text, trigger))
        } else {
            let n_pos = n_offset.expect("checked Some by !next_is_trigger arm");
            let text = self.flush_text(n_pos);
            let nl = Token::Newline { pos: n_pos };
            self.text_start = n_pos + 1;
            self.n_idx += 1;
            Some(self.pair_text_then(text, nl))
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        // Lower bound 0 (we may be at EOF after a final flush). Upper
        // bound is at most one event per remaining trigger / newline
        // plus interleaved text + the pending slot. For consumers this
        // is mainly a `Vec::with_capacity` hint, so over-estimating
        // is cheap.
        let triggers_left = self.trigger_offsets.len().saturating_sub(self.t_idx);
        let newlines_left = self.newline_offsets.len().saturating_sub(self.n_idx);
        // Each event contributes at most 2 tokens (text + structural).
        let upper = (triggers_left + newlines_left) * 2 + usize::from(self.pending.is_some()) + 1;
        (0, Some(upper))
    }
}

/// Look at the 3-byte window at `pos` and return its [`TriggerKind`].
/// Caller guarantees `pos + 3 <= bytes.len()` and that the window is
/// in fact a recognised trigger (the scanner's contract).
#[inline]
fn trigger_kind_at(bytes: &[u8], pos: usize) -> TriggerKind {
    let window: [u8; 3] = [bytes[pos], bytes[pos + 1], bytes[pos + 2]];
    classify_trigger_bytes(window).expect("scanner only emits classified positions")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(src: &str) -> Vec<Token> {
        tokenize(src).collect()
    }

    fn triggers(tokens: &[Token]) -> Vec<TriggerKind> {
        tokens
            .iter()
            .filter_map(|t| match t {
                Token::Trigger { kind, .. } => Some(*kind),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn plain_text_is_one_text_token() {
        let toks = collect("hello world こんにちは");
        assert_eq!(toks.len(), 1);
        match &toks[0] {
            Token::Text { range } => {
                assert_eq!(range.start, 0);
                assert_eq!(range.end as usize, "hello world こんにちは".len());
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn empty_input_yields_no_tokens() {
        assert!(collect("").is_empty());
    }

    #[test]
    fn single_newline_emits_newline_token() {
        let toks = collect("\n");
        assert_eq!(toks.len(), 1);
        assert!(matches!(toks[0], Token::Newline { pos: 0 }));
    }

    #[test]
    fn explicit_ruby_emits_bar_open_close() {
        let toks = collect("a｜漢字《かんじ》b");
        let kinds = triggers(&toks);
        assert_eq!(
            kinds,
            vec![
                TriggerKind::Bar,
                TriggerKind::RubyOpen,
                TriggerKind::RubyClose,
            ]
        );
    }

    #[test]
    fn double_bouten_brackets_merge_into_double_triggers() {
        let toks = collect("《《強調》》");
        let kinds = triggers(&toks);
        assert_eq!(
            kinds,
            vec![TriggerKind::DoubleRubyOpen, TriggerKind::DoubleRubyClose,]
        );
    }

    #[test]
    fn bracket_annotation_emits_each_component_separately() {
        let toks = collect("［＃改ページ］");
        let kinds = triggers(&toks);
        assert_eq!(
            kinds,
            vec![
                TriggerKind::BracketOpen,
                TriggerKind::Hash,
                TriggerKind::BracketClose,
            ]
        );
    }

    #[test]
    fn gaiji_ref_mark_is_emitted() {
        let toks = collect("※［＃「木」、1-2-3］");
        let kinds = triggers(&toks);
        assert_eq!(
            kinds,
            vec![
                TriggerKind::RefMark,
                TriggerKind::BracketOpen,
                TriggerKind::Hash,
                TriggerKind::QuoteOpen,
                TriggerKind::QuoteClose,
                TriggerKind::BracketClose,
            ]
        );
    }

    #[test]
    fn tortoise_brackets_emit_dedicated_triggers() {
        let toks = collect("〔e^〕");
        let kinds = triggers(&toks);
        assert_eq!(
            kinds,
            vec![TriggerKind::TortoiseOpen, TriggerKind::TortoiseClose]
        );
    }

    #[test]
    fn text_between_triggers_is_preserved() {
        let toks = collect("a｜b《c》d");
        let text_ranges: Vec<Span> = toks
            .iter()
            .filter_map(|t| match t {
                Token::Text { range } => Some(*range),
                _ => None,
            })
            .collect();
        // "a"(0..1) before ｜, "b"(4..5) between ｜ and 《, "c"(8..9), "d"(12..13).
        assert_eq!(text_ranges.len(), 4);
        assert_eq!(text_ranges[0], Span::new(0, 1));
        assert_eq!(text_ranges[1], Span::new(4, 5));
        assert_eq!(text_ranges[2], Span::new(8, 9));
        assert_eq!(text_ranges[3], Span::new(12, 13));
    }

    #[test]
    fn adjacent_triggers_produce_no_empty_text_tokens() {
        let toks = collect("｜《》");
        for tok in &toks {
            if let Token::Text { range } = tok {
                assert!(
                    range.end > range.start,
                    "empty Text token leaked into stream: {tok:?}"
                );
            }
        }
    }

    #[test]
    fn newline_is_its_own_token_between_text_runs() {
        let toks = collect("line1\nline2");
        assert_eq!(toks.len(), 3);
        match &toks[0] {
            Token::Text { range } => assert_eq!(*range, Span::new(0, 5)),
            other => panic!("expected Text, got {other:?}"),
        }
        assert!(matches!(toks[1], Token::Newline { pos: 5 }));
        match &toks[2] {
            Token::Text { range } => assert_eq!(*range, Span::new(6, 11)),
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn trigger_span_covers_all_constituent_bytes() {
        let toks = collect("《《ab》》");
        let open_span = toks
            .iter()
            .find_map(|t| match t {
                Token::Trigger {
                    kind: TriggerKind::DoubleRubyOpen,
                    span,
                } => Some(*span),
                _ => None,
            })
            .expect("DoubleRubyOpen present");
        // Double《 → 6 bytes starting at 0.
        assert_eq!(open_span, Span::new(0, 6));
    }
}
