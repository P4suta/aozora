#![expect(
    clippy::panic,
    reason = "Sentinel discriminants are compile-time Unicode scalar constants"
)]

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Sentinel {
    Inline = 0xE001,
    BlockLeaf = 0xE002,
    BlockOpen = 0xE003,
    BlockClose = 0xE004,
}

impl Sentinel {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 4] = [
        Self::Inline,
        Self::BlockLeaf,
        Self::BlockOpen,
        Self::BlockClose,
    ];

    #[must_use]
    pub(crate) const fn as_char(self) -> char {
        match char::from_u32(self as u32) {
            Some(c) => c,
            None => panic!("Sentinel discriminant must be a valid Unicode scalar"),
        }
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn from_char(c: char) -> Option<Self> {
        match c as u32 {
            0xE001 => Some(Self::Inline),
            0xE002 => Some(Self::BlockLeaf),
            0xE003 => Some(Self::BlockOpen),
            0xE004 => Some(Self::BlockClose),
            _ => None,
        }
    }
}

pub(crate) const INLINE_SENTINEL: char = Sentinel::Inline.as_char();

pub(crate) const BLOCK_LEAF_SENTINEL: char = Sentinel::BlockLeaf.as_char();

pub(crate) const BLOCK_OPEN_SENTINEL: char = Sentinel::BlockOpen.as_char();

pub(crate) const BLOCK_CLOSE_SENTINEL: char = Sentinel::BlockClose.as_char();

#[cfg(test)]
pub(crate) const ALL_SENTINELS: [char; 4] = [
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
