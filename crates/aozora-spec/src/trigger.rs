//! Trigger character classification.
//!
//! Aozora notation uses 11 distinct delimiter characters:
//!
//! | char | role                          | UTF-8 bytes |
//! |------|-------------------------------|-------------|
//! | `｜` | explicit ruby-base delimiter  | EF BD 9C    |
//! | `《` | ruby reading open             | E3 80 8A    |
//! | `》` | ruby reading close            | E3 80 8B    |
//! | `［` | bracket open                  | EF BC BB    |
//! | `］` | bracket close                 | EF BC BD    |
//! | `＃` | annotation keyword marker     | EF BC 83    |
//! | `※` | reference mark (gaiji prefix) | E2 80 BB    |
//! | `〔` | tortoise-shell open           | E3 80 94    |
//! | `〕` | tortoise-shell close          | E3 80 95    |
//! | `「` | corner-bracket open           | E3 80 8C    |
//! | `」` | corner-bracket close          | E3 80 8D    |
//!
//! Every trigger is a 3-byte UTF-8 BMP character. The leading byte is
//! one of `{0xE2, 0xE3, 0xEF}` — a fact the SIMD scanner exploits to
//! bulk-skip the 99.5% of source bytes that are not trigger candidates.
//!
//! Two double-character triggers (`《《`, `》》`) are merged into single
//! [`TriggerKind`] values by the lexer's structuring layer; the
//! per-byte classifier here only knows about the singletons.

use phf::phf_map;

/// Classification of a single trigger character (or merged double).
///
/// Single-character triggers cover 3 source bytes; the merged
/// `DoubleRubyOpen` / `DoubleRubyClose` cover 6.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum TriggerKind {
    /// `｜` (U+FF5C). Explicit ruby-base delimiter.
    Bar,

    /// `《` (U+300A). Ruby-reading open.
    RubyOpen,
    /// `》` (U+300B). Ruby-reading close.
    RubyClose,

    /// `《《` — two consecutive U+300A. Double-bracket bouten open.
    DoubleRubyOpen,
    /// `》》` — two consecutive U+300B. Double-bracket bouten close.
    DoubleRubyClose,

    /// `［` (U+FF3B). Square bracket open.
    BracketOpen,
    /// `］` (U+FF3D). Square bracket close.
    BracketClose,

    /// `＃` (U+FF03). Annotation keyword marker (meaningful after `［`).
    Hash,

    /// `※` (U+203B). Reference mark — prefix of a gaiji annotation.
    RefMark,

    /// `〔` (U+3014). Tortoise-shell bracket open.
    TortoiseOpen,
    /// `〕` (U+3015). Tortoise-shell bracket close.
    TortoiseClose,

    /// `「` (U+300C). Corner bracket open.
    QuoteOpen,
    /// `」` (U+300D). Corner bracket close.
    QuoteClose,
}

impl TriggerKind {
    /// Byte length of the canonical source form of this trigger in UTF-8.
    /// All single-character triggers are BMP codepoints encoded as 3
    /// UTF-8 bytes; the merged `DoubleRuby*` variants cover 6.
    #[must_use]
    pub const fn source_byte_len(self) -> u32 {
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

/// Compile-time perfect-hash table from a single-character trigger's
/// 3-byte UTF-8 sequence to its [`TriggerKind`].
///
/// Used by the SIMD-driven scanner: candidates produced by the
/// bit-parallel scan are 3-byte windows that need precise classification
/// in branch-predictable form. `phf::Map::get` is branch-free O(1)
/// (one multiply + one modulo + one compare), strictly better than a
/// `match` chain for this hot path. Double-character triggers
/// (`《《` / `》》`) are recognised at the structuring layer by
/// look-ahead, so they do not appear here.
static SINGLE_TRIGGER_TABLE: phf::Map<[u8; 3], TriggerKind> = phf_map! {
    [0xEFu8, 0xBDu8, 0x9Cu8] => TriggerKind::Bar,           // ｜ (U+FF5C)
    [0xE3u8, 0x80u8, 0x8Au8] => TriggerKind::RubyOpen,      // 《
    [0xE3u8, 0x80u8, 0x8Bu8] => TriggerKind::RubyClose,     // 》
    [0xEFu8, 0xBCu8, 0xBBu8] => TriggerKind::BracketOpen,   // ［
    [0xEFu8, 0xBCu8, 0xBDu8] => TriggerKind::BracketClose,  // ］
    [0xEFu8, 0xBCu8, 0x83u8] => TriggerKind::Hash,          // ＃
    [0xE2u8, 0x80u8, 0xBBu8] => TriggerKind::RefMark,       // ※
    [0xE3u8, 0x80u8, 0x94u8] => TriggerKind::TortoiseOpen,  // 〔
    [0xE3u8, 0x80u8, 0x95u8] => TriggerKind::TortoiseClose, // 〕
    [0xE3u8, 0x80u8, 0x8Cu8] => TriggerKind::QuoteOpen,     // 「
    [0xE3u8, 0x80u8, 0x8Du8] => TriggerKind::QuoteClose,    // 」
};

/// Classify a 3-byte UTF-8 window as a single-character trigger.
///
/// Returns `None` when the window is not a recognised trigger. Callers
/// must re-examine the window with their own state for `《《` / `》》`
/// merging.
///
/// Takes the window by value: a 3-byte array fits in a single 64-bit
/// register, so passing by value is strictly cheaper than the indirect
/// reference clippy's `trivially_copy_pass_by_ref` lint flags.
#[inline]
#[must_use]
pub fn classify_trigger_bytes(window: [u8; 3]) -> Option<TriggerKind> {
    SINGLE_TRIGGER_TABLE.get(&window).copied()
}

/// Set of UTF-8 leading bytes that may begin a trigger character.
/// The SIMD scanner uses this set to mask candidate positions before
/// running [`classify_trigger_bytes`] for precise classification.
pub const TRIGGER_LEADING_BYTES: [u8; 3] = [0xE2, 0xE3, 0xEF];

/// Set of UTF-8 *middle* bytes (2nd byte of the trigram) covering
/// every trigger character.
///
/// ADR-0015 found this set is ~4× sparser than [`TRIGGER_LEADING_BYTES`]
/// on Japanese text and used by the structural-bitmap scan strategy.
pub const TRIGGER_MIDDLE_BYTES: [u8; 3] = [0x80, 0xBC, 0xBD];

/// All 11 single-character trigger trigrams as raw UTF-8 byte arrays.
///
/// In PHF-table iteration order. Consumed by the multi-pattern scan
/// backends (Teddy, multi-pattern DFA — see ADR-0015) which need the
/// patterns directly rather than going through `classify_trigger_bytes`.
///
/// The accompanying test [`tests::all_trigger_trigrams_match_phf`]
/// asserts that every entry round-trips through the PHF, so adding /
/// removing a trigger keeps this list and the PHF in sync.
pub const ALL_TRIGGER_TRIGRAMS: [[u8; 3]; 11] = [
    [0xEF, 0xBD, 0x9C], // ｜ Bar
    [0xE3, 0x80, 0x8A], // 《 RubyOpen
    [0xE3, 0x80, 0x8B], // 》 RubyClose
    [0xEF, 0xBC, 0xBB], // ［ BracketOpen
    [0xEF, 0xBC, 0xBD], // ］ BracketClose
    [0xEF, 0xBC, 0x83], // ＃ Hash
    [0xE2, 0x80, 0xBB], // ※ RefMark
    [0xE3, 0x80, 0x94], // 〔 TortoiseOpen
    [0xE3, 0x80, 0x95], // 〕 TortoiseClose
    [0xE3, 0x80, 0x8C], // 「 QuoteOpen
    [0xE3, 0x80, 0x8D], // 」 QuoteClose
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_char_trigger_byte_lens_match_utf8() {
        // For each single-character variant, look up via PHF and assert
        // the encoded length is 3.
        for kind in [
            TriggerKind::Bar,
            TriggerKind::RubyOpen,
            TriggerKind::RubyClose,
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
    fn double_triggers_are_six_bytes() {
        assert_eq!(TriggerKind::DoubleRubyOpen.source_byte_len(), 6);
        assert_eq!(TriggerKind::DoubleRubyClose.source_byte_len(), 6);
    }

    #[test]
    fn classify_trigger_bytes_recognises_each_singleton() {
        let cases: &[(&str, TriggerKind)] = &[
            ("｜", TriggerKind::Bar),
            ("《", TriggerKind::RubyOpen),
            ("》", TriggerKind::RubyClose),
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
        // Plain hiragana 'あ' (U+3042 → E3 81 82) is *not* a trigger,
        // even though its leading byte is one of the candidate set.
        let bytes = "あ".as_bytes();
        let window: [u8; 3] = [bytes[0], bytes[1], bytes[2]];
        assert_eq!(classify_trigger_bytes(window), None);
    }

    #[test]
    fn trigger_leading_bytes_are_complete_for_known_triggers() {
        // Every entry in the PHF table starts with one of the listed
        // leading bytes. If a future trigger character is ever added
        // outside this set this test will fail and force the SIMD
        // scanner mask to be updated alongside.
        for entry_key in SINGLE_TRIGGER_TABLE.keys() {
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
        for entry_key in SINGLE_TRIGGER_TABLE.keys() {
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
            let used = SINGLE_TRIGGER_TABLE.keys().any(|k| k[1] == b);
            assert!(used, "middle byte {b:#04X} listed but unused");
        }
    }

    #[test]
    fn all_trigger_trigrams_match_phf() {
        // ALL_TRIGGER_TRIGRAMS must be the exact set of PHF keys
        // (no missing entries; no duplicates; no stale entries).
        // Forward: every PHF key is in the array.
        for k in SINGLE_TRIGGER_TABLE.keys() {
            assert!(
                ALL_TRIGGER_TRIGRAMS.iter().any(|t| t == k),
                "PHF key {k:?} not in ALL_TRIGGER_TRIGRAMS"
            );
        }
        // Reverse: every array entry classifies via PHF and has the
        // expected length of 11 — these are the load-bearing
        // multi-pattern inputs to the Teddy / DFA scanners.
        assert_eq!(ALL_TRIGGER_TRIGRAMS.len(), SINGLE_TRIGGER_TABLE.len());
        for trigram in &ALL_TRIGGER_TRIGRAMS {
            assert!(
                classify_trigger_bytes(*trigram).is_some(),
                "ALL_TRIGGER_TRIGRAMS entry {trigram:?} doesn't classify"
            );
        }
    }

    #[test]
    fn trigger_leading_bytes_has_no_redundant_entries() {
        // Conversely: every byte in the leading set is actually used
        // by at least one trigger. Catches stale entries.
        for &b in &TRIGGER_LEADING_BYTES {
            let used = SINGLE_TRIGGER_TABLE.keys().any(|k| k[0] == b);
            assert!(
                used,
                "leading byte {b:#04X} listed in TRIGGER_LEADING_BYTES \
                 but no trigger uses it"
            );
        }
    }
}
