//! Lexer token types.
//!
//! Phase 1 emits a `Vec<Token>` where each token is either a plain
//! [`Token::Text`] range (a run of source bytes between triggers) or a
//! [`Token::Trigger`] carrying the specific delimiter kind that caused
//! the break. Phase 2 consumes this stream and applies balanced-stack
//! pairing to build structured events.
//!
//! [`TriggerKind`] now lives in [`aozora_spec::TriggerKind`]; it is
//! re-exported here for backward compatibility through the 0.1 → 0.2
//! transition.

use aozora_syntax::Span;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_char_trigger_byte_lens_match_utf8() {
        // Sanity that the re-export still works the same.
        assert_eq!(TriggerKind::Bar.source_byte_len(), 3);
        assert_eq!(TriggerKind::DoubleRubyOpen.source_byte_len(), 6);
    }
}
