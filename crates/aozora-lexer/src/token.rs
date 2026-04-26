//! Lexer token types + arena-backed Structure-of-Arrays storage.
//!
//! The public value type [`Token`] models one Phase 1 lexer event:
//! either plain text between triggers, a delimiter trigger, or a
//! newline. Phase 2 used to consume `Vec<Token>`; M-2 / ADR-0019
//! replaces that with [`TokenStream`], a 4-column `SoA` layout backed
//! by `bumpalo::collections::Vec` inside the parse arena. Each
//! column stores a single type so the hot loop in `pair_in` reads
//! the **tag column alone** (1 byte per token) until it actually
//! needs payload — a cache-line per 64 tokens vs a cache-line per
//! ~5 tokens for the 12-byte enum.
//!
//! Why type-safe 4 columns and not a single packed `KindByte(u8)`:
//! columns store one type each, so no `unsafe { transmute }` is
//! needed to interpret payload bytes. Cost: one wasted byte per
//! token in the unused-payload column (a `Newline` row carries a
//! dummy `TriggerKind`). Net storage: 10-11 bytes / token vs the
//! enum's 12 bytes — modest packing win, large tag-density win.
//!
//! [`TriggerKind`] now lives in [`aozora_spec::TriggerKind`]; it is
//! re-exported here for backward compatibility through the 0.1 → 0.2
//! transition.

use aozora_syntax::Span;
use aozora_syntax::borrowed::Arena;
use bumpalo::collections::Vec as BumpVec;

pub use aozora_spec::TriggerKind;

/// A single lexer event.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Token {
    /// Text between triggers. `range` is a byte-offset span in the
    /// sanitized source (Phase 0 output). May be empty if two triggers
    /// are adjacent.
    Text { range: Span },

    /// A delimiter character. `pos` is the start byte offset of the
    /// token in the sanitized source; `kind` carries its role. For
    /// multi-character triggers (`《《`, `》》`, `［＃`) the span covers
    /// all constituent characters.
    Trigger { kind: TriggerKind, span: Span },

    /// Line-feed (`\n`). Emitted as its own token rather than folded
    /// into the surrounding Text because line-structure matters for
    /// block-level container recognition (Phase 2 pairs block-opener /
    /// block-closer lines by position).
    Newline { pos: u32 },
}

/// Storage tag for [`TokenStream`].
///
/// One byte per token, scanned densely in Phase 2's hot loop
/// (`pair_in`) to dispatch to the per-variant handler. The
/// discriminant order is irrelevant — pattern matches are exhaustive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenTag {
    Text,
    Trigger,
    Newline,
}

/// Arena-backed Structure-of-Arrays storage for a Phase 1 token stream.
///
/// Materialised by [`crate::tokenize_in`]; consumed by Phase 2's
/// [`crate::pair_in`] (and any other downstream that wants to walk
/// the stream).
///
/// ## Storage layout
///
/// | Column | Type | Bytes / elem | Populated when |
/// |---|---|---:|---|
/// | `tags` | [`TokenTag`] | 1 | always |
/// | `spans` | [`Span`] | 8 | always (Newline rows store `Span(pos, pos + 1)`) |
/// | `trigger_kinds` | [`TriggerKind`] | 1 | only `tag == Trigger`; other rows carry a dummy value |
///
/// Total: ~10 bytes / token. The `tag` column is dense enough that
/// 64 token tags fit in one cache line; pure tag-scan loops touch
/// 1 cache line per 64 tokens vs 1 cache line per ~5 tokens for
/// the 12-byte enum layout. Variant payloads are read out-of-band
/// only when the tag selects them.
#[derive(Debug)]
pub struct TokenStream<'a> {
    tags: BumpVec<'a, TokenTag>,
    spans: BumpVec<'a, Span>,
    trigger_kinds: BumpVec<'a, TriggerKind>,
}

impl<'a> TokenStream<'a> {
    /// Empty stream backed by `arena`. Capacity hint avoids the
    /// re-grow path on dense docs (one bump-pointer-rewind per column
    /// per re-grow).
    #[must_use]
    pub fn with_capacity_in(cap: usize, arena: &'a Arena) -> Self {
        let bump = arena.bump();
        Self {
            tags: BumpVec::with_capacity_in(cap, bump),
            spans: BumpVec::with_capacity_in(cap, bump),
            trigger_kinds: BumpVec::with_capacity_in(cap, bump),
        }
    }

    /// Append a [`Token::Text`] row.
    #[inline]
    pub fn push_text(&mut self, range: Span) {
        self.tags.push(TokenTag::Text);
        self.spans.push(range);
        // Dummy payload — never read (tag-gated).
        self.trigger_kinds.push(TriggerKind::Bar);
    }

    /// Append a [`Token::Trigger`] row.
    #[inline]
    pub fn push_trigger(&mut self, kind: TriggerKind, span: Span) {
        self.tags.push(TokenTag::Trigger);
        self.spans.push(span);
        self.trigger_kinds.push(kind);
    }

    /// Append a [`Token::Newline`] row. Internally stored as
    /// `Span(pos, pos + 1)` — a one-byte span — so the spans column
    /// stays uniform.
    #[inline]
    pub fn push_newline(&mut self, pos: u32) {
        self.tags.push(TokenTag::Newline);
        self.spans.push(Span::new(pos, pos + 1));
        self.trigger_kinds.push(TriggerKind::Bar);
    }

    /// Total number of tokens.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tags.len()
    }

    /// True if the stream contains no tokens.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    /// Tag at index `i`. Used by hot tag-scan loops in Phase 2.
    ///
    /// # Panics
    ///
    /// Panics if `i >= self.len()`.
    #[inline]
    #[must_use]
    pub fn tag_at(&self, i: usize) -> TokenTag {
        self.tags[i]
    }

    /// Span at index `i`. For `TokenTag::Newline` rows this is
    /// `Span(pos, pos + 1)`; callers reading newline position should
    /// use [`Self::newline_pos_at`] for clarity.
    #[inline]
    #[must_use]
    pub fn span_at(&self, i: usize) -> Span {
        self.spans[i]
    }

    /// Trigger kind at index `i`. Caller must have verified
    /// `tag_at(i) == TokenTag::Trigger` first; reads on other rows
    /// return the dummy value pushed at construction time.
    #[inline]
    #[must_use]
    pub fn trigger_kind_at(&self, i: usize) -> TriggerKind {
        self.trigger_kinds[i]
    }

    /// Byte position of a `Newline` row. Caller must have verified
    /// `tag_at(i) == TokenTag::Newline`.
    #[inline]
    #[must_use]
    pub fn newline_pos_at(&self, i: usize) -> u32 {
        self.spans[i].start
    }

    /// Iterator over the stream as `Token` values, reconstructing
    /// each variant from the columns. Use this for legacy
    /// consumers and tests that expect `IntoIterator<Item = Token>`.
    /// Hot Phase 2 consumers should use [`Self::tag_at`] + the
    /// per-column accessors directly to keep the tag-density win.
    pub fn iter(&self) -> impl Iterator<Item = Token> + '_ {
        (0..self.len()).map(move |i| match self.tag_at(i) {
            TokenTag::Text => Token::Text {
                range: self.span_at(i),
            },
            TokenTag::Trigger => Token::Trigger {
                kind: self.trigger_kind_at(i),
                span: self.span_at(i),
            },
            TokenTag::Newline => Token::Newline {
                pos: self.newline_pos_at(i),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_char_trigger_byte_lens_match_utf8() {
        // Sanity that the re-export still works the same.
        assert_eq!(TriggerKind::Bar.source_byte_len(), 3);
        assert_eq!(TriggerKind::DoubleRubyOpen.source_byte_len(), 6);
    }

    #[test]
    fn token_stream_round_trips_via_iter() {
        let arena = Arena::new();
        let mut s = TokenStream::with_capacity_in(8, &arena);
        s.push_text(Span::new(0, 5));
        s.push_trigger(TriggerKind::Bar, Span::new(5, 8));
        s.push_newline(8);
        s.push_trigger(TriggerKind::RubyOpen, Span::new(9, 12));

        let collected: Vec<Token> = s.iter().collect();
        assert_eq!(
            collected,
            vec![
                Token::Text {
                    range: Span::new(0, 5)
                },
                Token::Trigger {
                    kind: TriggerKind::Bar,
                    span: Span::new(5, 8)
                },
                Token::Newline { pos: 8 },
                Token::Trigger {
                    kind: TriggerKind::RubyOpen,
                    span: Span::new(9, 12)
                },
            ]
        );
        assert_eq!(s.len(), 4);
    }

    #[test]
    fn token_stream_tag_only_scan() {
        let arena = Arena::new();
        let mut s = TokenStream::with_capacity_in(8, &arena);
        s.push_text(Span::new(0, 1));
        s.push_newline(1);
        s.push_trigger(TriggerKind::Bar, Span::new(2, 5));
        let tags: Vec<TokenTag> = (0..s.len()).map(|i| s.tag_at(i)).collect();
        assert_eq!(
            tags,
            vec![TokenTag::Text, TokenTag::Newline, TokenTag::Trigger]
        );
    }
}
