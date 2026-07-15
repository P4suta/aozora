//! Trigger character classification.
//!
//! Aozora notation uses 13 distinct delimiter characters:
//!
//! | char | role                          | UTF-8 bytes |
//! |------|-------------------------------|-------------|
//! | `｜` | explicit ruby-base delimiter  | EF BD 9C    |
//! | `《` | ruby reading open             | E3 80 8A    |
//! | `》` | ruby reading close            | E3 80 8B    |
//! | `≪` | double-angle quotation open   | E2 89 AA    |
//! | `≫` | double-angle quotation close  | E2 89 AB    |
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
//! `≪`/`≫` (U+226A/U+226B) are the aozora input encoding for a 底本's
//! double-angle brackets `《`/`》` (which would otherwise collide with the
//! ruby delimiters U+300A/U+300B); a renderer displays them back as
//! `《`/`》`. Being distinct codepoints, they need no look-ahead merge.

/// Classification of a single trigger character.
///
/// Every trigger is a single BMP codepoint covering 3 source bytes.
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

    /// `≪` (U+226A). Double-angle quotation open — the aozora input
    /// encoding for a 底本 `《`; a renderer displays it as `《`.
    AngleQuoteOpen,
    /// `≫` (U+226B). Double-angle quotation close — input encoding for a
    /// 底本 `》`; a renderer displays it as `》`.
    AngleQuoteClose,

    /// `［` (U+FF3B). Square bracket open.
    BracketOpen,
    /// `］` (U+FF3D). Square bracket close.
    BracketClose,

    /// `＃` (U+FF03). Directive keyword marker (meaningful after `［`).
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
    /// Every trigger is a BMP codepoint encoded as 3 UTF-8 bytes.
    #[must_use]
    pub const fn source_byte_len(self) -> u32 {
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
            | Self::QuoteClose => 3,
        }
    }
}

/// Classify a 3-byte UTF-8 window as a single-character trigger.
///
/// Returns `None` when the window is not a recognised trigger.
///
/// ## Why a direct `match`, not a perfect-hash map
///
/// This sits in the lexer's innermost loop: the SIMD scanner emits a
/// candidate position for every leading byte in `{0xE2, 0xE3, 0xEF}`,
/// and each one is classified here. An earlier version backed this with
/// a `phf::Map<[u8; 3], _>` on the assumption that a perfect hash is
/// "branch-free O(1), strictly better than a `match` chain". A
/// flamegraph of a ruby-dense document disproved that: `phf::Map::get`
/// hashes the key with `SipHash`-1-3, and a `SipHash` over three bytes
/// costs far more than a handful of byte comparisons over a set this
/// small — the hash alone accounted for ≈0.8 % of total render time
/// (≈5 % of the parser's own time).
///
/// The 13 trigrams carry a trivial discriminator — the leading byte
/// splits them into `0xE2` (three: ※ ≪ ≫, by middle + trailing byte),
/// `0xE3` (six, by the trailing byte) and `0xEF` (four, by the middle
/// and trailing byte) — so an exhaustive `match` lowers to a small
/// comparison tree with no hashing at all. The `tests` module pins
/// this `match` against [`ALL_TRIGGER_TRIGRAMS`], exhaustively over
/// the candidate leading-byte space, so the two cannot silently drift.
///
/// Takes the window by value: a 3-byte array fits in a single 64-bit
/// register, so passing by value is strictly cheaper than the indirect
/// reference clippy's `trivially_copy_pass_by_ref` lint flags.
#[inline]
#[must_use]
pub fn classify_trigger_bytes(window: [u8; 3]) -> Option<TriggerKind> {
    Some(match window {
        [0xE2, 0x80, 0xBB] => TriggerKind::RefMark,         // ※
        [0xE2, 0x89, 0xAA] => TriggerKind::AngleQuoteOpen,  // ≪
        [0xE2, 0x89, 0xAB] => TriggerKind::AngleQuoteClose, // ≫
        [0xE3, 0x80, 0x8A] => TriggerKind::RubyOpen,        // 《
        [0xE3, 0x80, 0x8B] => TriggerKind::RubyClose,       // 》
        [0xE3, 0x80, 0x8C] => TriggerKind::QuoteOpen,       // 「
        [0xE3, 0x80, 0x8D] => TriggerKind::QuoteClose,      // 」
        [0xE3, 0x80, 0x94] => TriggerKind::TortoiseOpen,    // 〔
        [0xE3, 0x80, 0x95] => TriggerKind::TortoiseClose,   // 〕
        [0xEF, 0xBC, 0x83] => TriggerKind::Hash,            // ＃
        [0xEF, 0xBC, 0xBB] => TriggerKind::BracketOpen,     // ［
        [0xEF, 0xBC, 0xBD] => TriggerKind::BracketClose,    // ］
        [0xEF, 0xBD, 0x9C] => TriggerKind::Bar,             // ｜
        _ => return None,
    })
}

/// Set of UTF-8 leading bytes that may begin a trigger character.
/// The SIMD scanner uses this set to mask candidate positions before
/// running [`classify_trigger_bytes`] for precise classification.
pub const TRIGGER_LEADING_BYTES: [u8; 3] = [0xE2, 0xE3, 0xEF];

/// Set of UTF-8 *middle* bytes (2nd byte of the trigram) covering
/// every trigger character.
///
/// Empirically ~4× sparser than [`TRIGGER_LEADING_BYTES`] on Japanese
/// text; used by the structural-bitmap scan strategy.
pub const TRIGGER_MIDDLE_BYTES: [u8; 4] = [0x80, 0x89, 0xBC, 0xBD];

/// Every single-character trigger trigram as raw UTF-8 byte arrays —
/// one per [`TriggerKind`] variant.
///
/// In PHF-table iteration order. Consumed by the multi-pattern scan
/// backends (Teddy, multi-pattern DFA) which need the
/// patterns directly rather than going through `classify_trigger_bytes`.
///
/// The accompanying `tests::all_trigger_trigrams_match_phf` test
/// asserts that every entry round-trips through the PHF, so adding /
/// removing a trigger keeps this list and the PHF in sync.
pub const ALL_TRIGGER_TRIGRAMS: [[u8; 3]; 13] = [
    [0xEF, 0xBD, 0x9C], // ｜ Bar
    [0xE3, 0x80, 0x8A], // 《 RubyOpen
    [0xE3, 0x80, 0x8B], // 》 RubyClose
    [0xE2, 0x89, 0xAA], // ≪ AngleQuoteOpen
    [0xE2, 0x89, 0xAB], // ≫ AngleQuoteClose
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
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn single_char_trigger_byte_lens_match_utf8() {
        // For each single-character variant, look up via PHF and assert
        // the encoded length is 3.
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
        // Plain hiragana 'あ' (U+3042 → E3 81 82) is *not* a trigger,
        // even though its leading byte is one of the candidate set.
        let bytes = "あ".as_bytes();
        let window: [u8; 3] = [bytes[0], bytes[1], bytes[2]];
        assert_eq!(classify_trigger_bytes(window), None);
    }

    #[test]
    fn trigger_leading_bytes_are_complete_for_known_triggers() {
        // Every recognised trigram starts with one of the listed
        // leading bytes. If a future trigger character is ever added
        // outside this set this test will fail and force the SIMD
        // scanner mask to be updated alongside.
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
        // With the PHF gone, the `classify_trigger_bytes` match arms and
        // `ALL_TRIGGER_TRIGRAMS` are two hand-maintained copies of the
        // same 11-key set. This pins them so adding / removing a trigger
        // in one place without the other fails CI.
        //
        // Forward: every listed trigram classifies to a *distinct* kind.
        let mut kinds = HashSet::new();
        for trigram in &ALL_TRIGGER_TRIGRAMS {
            let kind = classify_trigger_bytes(*trigram)
                .unwrap_or_else(|| panic!("{trigram:?} listed but classifies to None"));
            assert!(kinds.insert(kind), "duplicate kind for {trigram:?}");
        }
        assert_eq!(kinds.len(), 13, "expected exactly 13 distinct triggers");
        assert_eq!(ALL_TRIGGER_TRIGRAMS.len(), 13);

        // Reverse: the match accepts *nothing* outside the array, swept
        // exhaustively over the candidate leading-byte space (the only
        // bytes the SIMD scanner ever feeds in — guarded complete by
        // `trigger_leading_bytes_are_complete_for_known_triggers`).
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
        // Conversely: every byte in the leading set is actually used
        // by at least one trigger. Catches stale entries.
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
