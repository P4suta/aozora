//! PUA sentinel codepoints reserved by the Aozora pipeline.
//!
//! The lexer rewrites every recognised Aozora construct into one of
//! these single-character sentinels in the normalized text it hands to
//! the downstream CommonMark parser. The placeholder registry maps each
//! sentinel position back to the originating [`crate::Span`] +
//! `AozoraNode`, so `post_process` can splice the construct back into
//! the AST after CommonMark parsing.
//!
//! All four sentinels live in the Unicode Private Use Area (`U+E000..U+F8FF`),
//! which is guaranteed to be unassigned and therefore safe to use as
//! application-internal markers. A pre-scan in Phase 0 (sanitize)
//! emits `Diagnostic::SourceContainsPua` if the source already
//! contains any of these codepoints — a future enhancement can fall
//! back to Unicode noncharacters (`U+FDD0..U+FDEF`) when collisions
//! become recurring.

/// Inline Aozora span (ruby / bouten / annotation / gaiji / TCY / kaeriten).
pub const INLINE_SENTINEL: char = '\u{E001}';

/// Block-leaf Aozora line (page break, section break, leaf indent, sashie).
pub const BLOCK_LEAF_SENTINEL: char = '\u{E002}';

/// Paired-container open line (e.g. `［＃ここから字下げ］`).
pub const BLOCK_OPEN_SENTINEL: char = '\u{E003}';

/// Paired-container close line (e.g. `［＃ここで字下げ終わり］`).
pub const BLOCK_CLOSE_SENTINEL: char = '\u{E004}';

/// All four sentinels in declaration order. Useful for collision scans
/// and exhaustive iteration.
pub const ALL_SENTINELS: [char; 4] = [
    INLINE_SENTINEL,
    BLOCK_LEAF_SENTINEL,
    BLOCK_OPEN_SENTINEL,
    BLOCK_CLOSE_SENTINEL,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentinels_are_in_pua_range() {
        for &c in &ALL_SENTINELS {
            let code = u32::from(c);
            assert!(
                (0xE000..=0xF8FF).contains(&code),
                "{c:?} ({code:#06X}) must lie in Unicode PUA"
            );
        }
    }

    #[test]
    fn sentinels_are_pairwise_distinct() {
        for (i, a) in ALL_SENTINELS.iter().enumerate() {
            for b in &ALL_SENTINELS[i + 1..] {
                assert_ne!(a, b, "sentinels must be pairwise distinct");
            }
        }
    }

    #[test]
    fn all_sentinels_constant_lists_every_named_constant() {
        // Defensive: if a new sentinel is ever added we want this test
        // to fail until ALL_SENTINELS is updated too.
        assert_eq!(ALL_SENTINELS.len(), 4);
        assert!(ALL_SENTINELS.contains(&INLINE_SENTINEL));
        assert!(ALL_SENTINELS.contains(&BLOCK_LEAF_SENTINEL));
        assert!(ALL_SENTINELS.contains(&BLOCK_OPEN_SENTINEL));
        assert!(ALL_SENTINELS.contains(&BLOCK_CLOSE_SENTINEL));
    }
}
