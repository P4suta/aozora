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
//!
//! # Source of truth
//!
//! [`Sentinel`] is the primary type: a `#[repr(u32)]` enum with one
//! variant per sentinel kind. The four legacy `pub const` `char`
//! values ([`INLINE_SENTINEL`] etc.) and the [`ALL_SENTINELS`] array
//! are now thin shims derived from the enum via
//! [`Sentinel::as_char`], so a new sentinel kind only needs adding to
//! the enum.

/// Sentinel kind tag.
///
/// Each variant matches a Private-Use-Area codepoint reserved by the
/// lexer. The discriminant doubles as the codepoint, so the
/// `as_char` / `from_char` projections compile to a noop / range
/// check respectively.
///
/// `#[repr(u32)]` pins the in-memory layout to the codepoint scalar,
/// matching Rust's `char` ABI for round-tripping cost. The variant
/// values stay inside the PUA range (`0xE000..=0xF8FF`); pinning the
/// 4-byte layout instead of `u8` lets the enum carry the codepoint
/// directly without a lookup table.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sentinel {
    /// Inline Aozora span (ruby / bouten / annotation / gaiji / TCY / kaeriten).
    Inline = 0xE001,
    /// Block-leaf Aozora line (page break, section break, leaf indent, sashie).
    BlockLeaf = 0xE002,
    /// Paired-container open line (e.g. `［＃ここから字下げ］`).
    BlockOpen = 0xE003,
    /// Paired-container close line (e.g. `［＃ここで字下げ終わり］`).
    BlockClose = 0xE004,
}

impl Sentinel {
    /// All sentinel kinds in declaration order. Useful for collision
    /// scans, exhaustive iteration, and round-trip property tests.
    pub const ALL: [Self; 4] = [
        Self::Inline,
        Self::BlockLeaf,
        Self::BlockOpen,
        Self::BlockClose,
    ];

    /// Codepoint for this sentinel kind. Compiles to a constant — the
    /// `#[repr(u32)]` discriminant is the codepoint scalar, so the
    /// transmute via `char::from_u32_unchecked` is a noop. We use the
    /// safe `char::from_u32` path here and rely on the variant
    /// discriminants being valid scalar values (they are: PUA is a
    /// proper subset of the scalar range and all four sentinels lie
    /// inside it).
    ///
    /// # Panics
    ///
    /// `panic!`s with a const-eval-friendly message if a future
    /// variant ever lands an invalid Unicode scalar discriminant.
    /// All current variants ([`Self::Inline`] / [`Self::BlockLeaf`] /
    /// [`Self::BlockOpen`] / [`Self::BlockClose`]) lie in the
    /// `U+E001..U+E004` PUA range and never trigger the panic; the
    /// arm exists to keep `#[repr(u32)]` discriminants honest at
    /// const-eval time.
    #[must_use]
    pub const fn as_char(self) -> char {
        // SAFETY-equivalent reasoning: each discriminant is a valid
        // scalar value (PUA codepoint), so `char::from_u32` returns
        // `Some` for every variant. We unwrap with a const-friendly
        // pattern that turns the panic into a const-eval error if a
        // future variant ever lands an invalid scalar.
        match char::from_u32(self as u32) {
            Some(c) => c,
            None => panic!("Sentinel discriminant must be a valid Unicode scalar"),
        }
    }

    /// Reverse of [`Sentinel::as_char`]. Returns `None` for any
    /// codepoint outside the four reserved sentinels.
    #[must_use]
    pub const fn from_char(c: char) -> Option<Self> {
        match c as u32 {
            0xE001 => Some(Self::Inline),
            0xE002 => Some(Self::BlockLeaf),
            0xE003 => Some(Self::BlockOpen),
            0xE004 => Some(Self::BlockClose),
            _ => None,
        }
    }
}

/// Inline Aozora span (ruby / bouten / annotation / gaiji / TCY / kaeriten).
///
/// Convenience shim — equivalent to [`Sentinel::Inline`]'s
/// [`as_char`](Sentinel::as_char).
pub const INLINE_SENTINEL: char = Sentinel::Inline.as_char();

/// Block-leaf Aozora line (page break, section break, leaf indent, sashie).
///
/// Convenience shim — equivalent to [`Sentinel::BlockLeaf`]'s
/// [`as_char`](Sentinel::as_char).
pub const BLOCK_LEAF_SENTINEL: char = Sentinel::BlockLeaf.as_char();

/// Paired-container open line (e.g. `［＃ここから字下げ］`).
///
/// Convenience shim — equivalent to [`Sentinel::BlockOpen`]'s
/// [`as_char`](Sentinel::as_char).
pub const BLOCK_OPEN_SENTINEL: char = Sentinel::BlockOpen.as_char();

/// Paired-container close line (e.g. `［＃ここで字下げ終わり］`).
///
/// Convenience shim — equivalent to [`Sentinel::BlockClose`]'s
/// [`as_char`](Sentinel::as_char).
pub const BLOCK_CLOSE_SENTINEL: char = Sentinel::BlockClose.as_char();

/// All four sentinels in declaration order.
///
/// Convenience shim — equivalent to mapping [`Sentinel::ALL`] through
/// [`Sentinel::as_char`].
pub const ALL_SENTINELS: [char; 4] = [
    Sentinel::Inline.as_char(),
    Sentinel::BlockLeaf.as_char(),
    Sentinel::BlockOpen.as_char(),
    Sentinel::BlockClose.as_char(),
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

    #[test]
    fn sentinel_round_trips_through_char_projection() {
        for &kind in &Sentinel::ALL {
            let c = kind.as_char();
            assert_eq!(Sentinel::from_char(c), Some(kind));
        }
    }

    #[test]
    fn sentinel_from_char_returns_none_for_non_sentinel() {
        for c in ['a', 'あ', '\u{E000}', '\u{E005}', '\u{F8FF}'] {
            assert_eq!(
                Sentinel::from_char(c),
                None,
                "non-sentinel codepoint {c:?} must not classify"
            );
        }
    }

    #[test]
    fn sentinel_const_shims_match_enum_projection() {
        assert_eq!(INLINE_SENTINEL, Sentinel::Inline.as_char());
        assert_eq!(BLOCK_LEAF_SENTINEL, Sentinel::BlockLeaf.as_char());
        assert_eq!(BLOCK_OPEN_SENTINEL, Sentinel::BlockOpen.as_char());
        assert_eq!(BLOCK_CLOSE_SENTINEL, Sentinel::BlockClose.as_char());
    }
}
