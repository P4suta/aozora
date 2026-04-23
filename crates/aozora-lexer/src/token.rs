//! Lexer token types.
//!
//! Phase 1 emits a `Vec<Token>` where each token is either a plain
//! [`Token::Text`] range (a run of source bytes between triggers) or a
//! [`Token::Trigger`] carrying the specific delimiter kind that caused
//! the break. Phase 2 consumes this stream and applies balanced-stack
//! pairing to build structured events.
//!
//! Triggers are Aozora Bunko notation's syntactic markers — the
//! characters that open/close ruby spans, bracket annotations, accent
//! segments, and so on. Plain CommonMark punctuation (`*`, `_`, `[`,
//! `` ` ``) is NOT a trigger here: the lexer leaves it for comrak to
//! interpret later.

use afm_syntax::Span;

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

/// Classification of a single [`Token::Trigger`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TriggerKind {
    /// `｜` (U+FF5C). Explicit ruby-base delimiter: everything until
    /// the following [`RubyOpen`](Self::RubyOpen) belongs to the base.
    Bar,

    /// `《` (U+300A). Ruby-reading open.
    RubyOpen,
    /// `》` (U+300B). Ruby-reading close.
    RubyClose,

    /// `《《` — two consecutive U+300A. Double-bracket bouten open.
    /// Phase 1 emits this when it sees two `《` in a row rather than
    /// two separate `RubyOpen`s, so phase 2 doesn't have to second-
    /// guess ambiguous `《《abc》》` spans.
    DoubleRubyOpen,
    /// `》》` — two consecutive U+300B. Double-bracket bouten close.
    DoubleRubyClose,

    /// `［` (U+FF3B). Square bracket open — start of an annotation
    /// when immediately followed by `＃`, otherwise plain text.
    BracketOpen,
    /// `］` (U+FF3D). Square bracket close.
    BracketClose,

    /// `＃` (U+FF03). Annotation keyword marker. Meaningful only
    /// immediately after `［`; emitted as its own token so Phase 2
    /// can validate the `［＃` opener shape.
    Hash,

    /// `※` (U+203B). Reference mark — prefix of a gaiji annotation
    /// (`※［＃…］`).
    RefMark,

    /// `〔` (U+3014). Tortoise-shell bracket open — delimits an
    /// accent-decomposition segment per ADR-0004.
    TortoiseOpen,
    /// `〕` (U+3015). Tortoise-shell bracket close.
    TortoiseClose,

    /// `「` (U+300C). Corner bracket open — used inside annotation
    /// bodies (`［＃「X」に傍点］`) to delimit quoted literals.
    QuoteOpen,
    /// `」` (U+300D). Corner bracket close.
    QuoteClose,
}

impl TriggerKind {
    /// Byte length of the canonical source form of this trigger in UTF-8.
    /// Used by Phase 1 to advance its cursor after emitting a trigger
    /// token. All triggers are BMP codepoints that encode to 3 bytes;
    /// double-character triggers (`《《`, `》》`) cover 6 bytes.
    #[must_use]
    pub const fn source_byte_len(self) -> u32 {
        // Every trigger character is in the BMP U+3000..U+FF5F range and
        // therefore encodes to exactly 3 UTF-8 bytes. Hard-coding the
        // constant instead of calling `char::len_utf8() as u32` keeps the
        // function `const` without any numeric cast (which would trip
        // `clippy::cast_possible_truncation` even for values that can't
        // actually truncate).
        match self {
            Self::Bar
            | Self::RubyOpen
            | Self::RubyClose
            | Self::BracketOpen
            | Self::BracketClose
            | Self::Hash
            | Self::RefMark
            | Self::TortoiseOpen
            | Self::TortoiseClose
            | Self::QuoteOpen
            | Self::QuoteClose => 3,
            Self::DoubleRubyOpen | Self::DoubleRubyClose => 6,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_char_trigger_byte_lens_match_utf8() {
        assert_eq!(TriggerKind::Bar.source_byte_len(), 3);
        assert_eq!(TriggerKind::RubyOpen.source_byte_len(), 3);
        assert_eq!(TriggerKind::RubyClose.source_byte_len(), 3);
        assert_eq!(TriggerKind::BracketOpen.source_byte_len(), 3);
        assert_eq!(TriggerKind::BracketClose.source_byte_len(), 3);
        assert_eq!(TriggerKind::Hash.source_byte_len(), 3);
        assert_eq!(TriggerKind::RefMark.source_byte_len(), 3);
        assert_eq!(TriggerKind::TortoiseOpen.source_byte_len(), 3);
        assert_eq!(TriggerKind::TortoiseClose.source_byte_len(), 3);
        assert_eq!(TriggerKind::QuoteOpen.source_byte_len(), 3);
        assert_eq!(TriggerKind::QuoteClose.source_byte_len(), 3);
    }

    #[test]
    fn double_triggers_are_six_bytes() {
        assert_eq!(TriggerKind::DoubleRubyOpen.source_byte_len(), 6);
        assert_eq!(TriggerKind::DoubleRubyClose.source_byte_len(), 6);
    }
}
