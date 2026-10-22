//! Classifies notation delimiters through [`classify_trigger_bytes`].

const TRIGGER_BYTE_LEN: u32 = 3;
const BAR_TRIGRAM: [u8; 3] = [0xEF, 0xBD, 0x9C];
const RUBY_OPEN_TRIGRAM: [u8; 3] = [0xE3, 0x80, 0x8A];
const RUBY_CLOSE_TRIGRAM: [u8; 3] = [0xE3, 0x80, 0x8B];
const ANGLE_QUOTE_OPEN_TRIGRAM: [u8; 3] = [0xE2, 0x89, 0xAA];
const ANGLE_QUOTE_CLOSE_TRIGRAM: [u8; 3] = [0xE2, 0x89, 0xAB];
const BRACKET_OPEN_TRIGRAM: [u8; 3] = [0xEF, 0xBC, 0xBB];
const BRACKET_CLOSE_TRIGRAM: [u8; 3] = [0xEF, 0xBC, 0xBD];
const HASH_TRIGRAM: [u8; 3] = [0xEF, 0xBC, 0x83];
const REF_MARK_TRIGRAM: [u8; 3] = [0xE2, 0x80, 0xBB];
const TORTOISE_OPEN_TRIGRAM: [u8; 3] = [0xE3, 0x80, 0x94];
const TORTOISE_CLOSE_TRIGRAM: [u8; 3] = [0xE3, 0x80, 0x95];
const QUOTE_OPEN_TRIGRAM: [u8; 3] = [0xE3, 0x80, 0x8C];
const QUOTE_CLOSE_TRIGRAM: [u8; 3] = [0xE3, 0x80, 0x8D];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub(crate) enum TriggerKind {
    /// Ruby-base delimiter `｜`.
    Bar,

    /// Ruby-reading opener `《`.
    RubyOpen,
    /// Ruby-reading closer `》`.
    RubyClose,

    /// Quotation opener `≪`, displayed as `《`.
    AngleQuoteOpen,
    /// Quotation closer `≫`, displayed as `》`.
    AngleQuoteClose,

    /// Annotation opener `［`.
    BracketOpen,
    /// Annotation closer `］`.
    BracketClose,

    /// Directive marker `＃`.
    Hash,

    /// Gaiji prefix `※`.
    RefMark,

    /// Accent-span opener `〔`.
    TortoiseOpen,
    /// Accent-span closer `〕`.
    TortoiseClose,

    /// Quoted-literal opener `「`.
    QuoteOpen,
    /// Quoted-literal closer `」`.
    QuoteClose,
}

impl TriggerKind {
    #[must_use]
    pub(crate) const fn source_byte_len(self) -> u32 {
        match self {
            Self::Bar
            | Self::RubyOpen
            | Self::RubyClose
            | Self::AngleQuoteOpen
            | Self::AngleQuoteClose
            | Self::BracketOpen
            | Self::BracketClose
            | Self::Hash
            | Self::RefMark
            | Self::TortoiseOpen
            | Self::TortoiseClose
            | Self::QuoteOpen
            | Self::QuoteClose => TRIGGER_BYTE_LEN,
        }
    }
}

#[inline]
#[must_use]
pub(crate) fn classify_trigger_bytes(window: [u8; 3]) -> Option<TriggerKind> {
    Some(match window {
        REF_MARK_TRIGRAM => TriggerKind::RefMark,
        ANGLE_QUOTE_OPEN_TRIGRAM => TriggerKind::AngleQuoteOpen,
        ANGLE_QUOTE_CLOSE_TRIGRAM => TriggerKind::AngleQuoteClose,
        RUBY_OPEN_TRIGRAM => TriggerKind::RubyOpen,
        RUBY_CLOSE_TRIGRAM => TriggerKind::RubyClose,
        QUOTE_OPEN_TRIGRAM => TriggerKind::QuoteOpen,
        QUOTE_CLOSE_TRIGRAM => TriggerKind::QuoteClose,
        TORTOISE_OPEN_TRIGRAM => TriggerKind::TortoiseOpen,
        TORTOISE_CLOSE_TRIGRAM => TriggerKind::TortoiseClose,
        HASH_TRIGRAM => TriggerKind::Hash,
        BRACKET_OPEN_TRIGRAM => TriggerKind::BracketOpen,
        BRACKET_CLOSE_TRIGRAM => TriggerKind::BracketClose,
        BAR_TRIGRAM => TriggerKind::Bar,
        _ => return None,
    })
}

pub(crate) const ALL_TRIGGER_TRIGRAMS: [[u8; 3]; 13] = [
    BAR_TRIGRAM,
    RUBY_OPEN_TRIGRAM,
    RUBY_CLOSE_TRIGRAM,
    ANGLE_QUOTE_OPEN_TRIGRAM,
    ANGLE_QUOTE_CLOSE_TRIGRAM,
    BRACKET_OPEN_TRIGRAM,
    BRACKET_CLOSE_TRIGRAM,
    HASH_TRIGRAM,
    REF_MARK_TRIGRAM,
    TORTOISE_OPEN_TRIGRAM,
    TORTOISE_CLOSE_TRIGRAM,
    QUOTE_OPEN_TRIGRAM,
    QUOTE_CLOSE_TRIGRAM,
];

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    const TRIGGER_LEADING_BYTES: [u8; 3] = [0xE2, 0xE3, 0xEF];

    const TRIGGER_MIDDLE_BYTES: [u8; 4] = [0x80, 0x89, 0xBC, 0xBD];

    #[test]
    fn single_char_trigger_byte_lens_match_utf8() {
        for kind in [
            TriggerKind::Bar,
            TriggerKind::RubyOpen,
            TriggerKind::RubyClose,
            TriggerKind::AngleQuoteOpen,
            TriggerKind::AngleQuoteClose,
            TriggerKind::BracketOpen,
            TriggerKind::BracketClose,
            TriggerKind::Hash,
            TriggerKind::RefMark,
            TriggerKind::TortoiseOpen,
            TriggerKind::TortoiseClose,
            TriggerKind::QuoteOpen,
            TriggerKind::QuoteClose,
        ] {
            assert_eq!(kind.source_byte_len(), 3, "{kind:?}");
        }
    }

    #[test]
    fn classify_trigger_bytes_recognises_each_singleton() {
        let cases: &[(&str, TriggerKind)] = &[
            ("｜", TriggerKind::Bar),
            ("《", TriggerKind::RubyOpen),
            ("》", TriggerKind::RubyClose),
            ("≪", TriggerKind::AngleQuoteOpen),
            ("≫", TriggerKind::AngleQuoteClose),
            ("［", TriggerKind::BracketOpen),
            ("］", TriggerKind::BracketClose),
            ("＃", TriggerKind::Hash),
            ("※", TriggerKind::RefMark),
            ("〔", TriggerKind::TortoiseOpen),
            ("〕", TriggerKind::TortoiseClose),
            ("「", TriggerKind::QuoteOpen),
            ("」", TriggerKind::QuoteClose),
        ];
        for (s, expected) in cases {
            let bytes = s.as_bytes();
            assert_eq!(bytes.len(), 3, "trigger {s:?} must be 3 UTF-8 bytes");
            let window: [u8; 3] = [bytes[0], bytes[1], bytes[2]];
            assert_eq!(
                classify_trigger_bytes(window),
                Some(*expected),
                "{s:?} should classify as {expected:?}"
            );
        }
    }

    #[test]
    fn classify_trigger_bytes_returns_none_for_non_trigger() {
        let bytes = "あ".as_bytes();
        let window: [u8; 3] = [bytes[0], bytes[1], bytes[2]];
        assert_eq!(classify_trigger_bytes(window), None);
    }

    #[test]
    fn trigger_leading_bytes_are_complete_for_known_triggers() {
        for entry_key in &ALL_TRIGGER_TRIGRAMS {
            assert!(
                TRIGGER_LEADING_BYTES.contains(&entry_key[0]),
                "trigger byte sequence {entry_key:?} starts with {:#04X} \
                 which is not in TRIGGER_LEADING_BYTES — \
                 update the SIMD scanner mask",
                entry_key[0]
            );
        }
    }

    #[test]
    fn trigger_middle_bytes_are_complete_for_known_triggers() {
        for entry_key in &ALL_TRIGGER_TRIGRAMS {
            assert!(
                TRIGGER_MIDDLE_BYTES.contains(&entry_key[1]),
                "trigger {entry_key:?} middle byte {:#04X} not in TRIGGER_MIDDLE_BYTES",
                entry_key[1]
            );
        }
    }

    #[test]
    fn trigger_middle_bytes_has_no_redundant_entries() {
        for &b in &TRIGGER_MIDDLE_BYTES {
            let used = ALL_TRIGGER_TRIGRAMS.iter().any(|k| k[1] == b);
            assert!(used, "middle byte {b:#04X} listed but unused");
        }
    }

    #[test]
    fn classify_match_and_trigram_array_cannot_drift() {
        let mut kinds = HashSet::new();
        for trigram in &ALL_TRIGGER_TRIGRAMS {
            let kind = classify_trigger_bytes(*trigram)
                .unwrap_or_else(|| panic!("{trigram:?} listed but classifies to None"));
            assert!(kinds.insert(kind), "duplicate kind for {trigram:?}");
        }
        assert_eq!(kinds.len(), 13, "expected exactly 13 distinct triggers");
        assert_eq!(ALL_TRIGGER_TRIGRAMS.len(), 13);

        for &b0 in &TRIGGER_LEADING_BYTES {
            for b1 in 0u8..=u8::MAX {
                for b2 in 0u8..=u8::MAX {
                    let window = [b0, b1, b2];
                    if classify_trigger_bytes(window).is_some() {
                        assert!(
                            ALL_TRIGGER_TRIGRAMS.contains(&window),
                            "match accepts {window:?}, absent from ALL_TRIGGER_TRIGRAMS"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn trigger_leading_bytes_has_no_redundant_entries() {
        for &b in &TRIGGER_LEADING_BYTES {
            let used = ALL_TRIGGER_TRIGRAMS.iter().any(|k| k[0] == b);
            assert!(
                used,
                "leading byte {b:#04X} listed in TRIGGER_LEADING_BYTES \
                 but no trigger uses it"
            );
        }
    }
}
