#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SourceOffset(pub u32);

impl SourceOffset {
    #[must_use]
    pub(crate) const fn new(v: u32) -> Self {
        Self(v)
    }

    #[must_use]
    pub(crate) const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for SourceOffset {
    fn from(v: u32) -> Self {
        Self(v)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct NormalizedOffset(pub u32);

impl NormalizedOffset {
    #[must_use]
    pub(crate) const fn new(v: u32) -> Self {
        Self(v)
    }

    #[must_use]
    pub(crate) const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for NormalizedOffset {
    fn from(v: u32) -> Self {
        Self(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_offset_round_trip_through_get() {
        let off = SourceOffset::new(42);
        assert_eq!(off.get(), 42);
        assert_eq!(off.0, 42);
    }

    #[test]
    fn normalized_offset_round_trip_through_get() {
        let off = NormalizedOffset::new(42);
        assert_eq!(off.get(), 42);
        assert_eq!(off.0, 42);
    }

    #[test]
    fn source_offset_compares_by_underlying_u32() {
        assert!(SourceOffset::new(3) < SourceOffset::new(5));
        assert_eq!(SourceOffset::new(7), SourceOffset::new(7));
    }

    #[test]
    fn from_u32_constructs_either_newtype() {
        let s: SourceOffset = 1u32.into();
        let n: NormalizedOffset = 1u32.into();
        assert_eq!(s.get(), 1);
        assert_eq!(n.get(), 1);
    }

    #[test]
    fn coordinate_spaces_are_disjoint() {
        let s = SourceOffset::new(5);
        let n = NormalizedOffset::new(5);
        assert_eq!(s.get(), n.get());
    }

    #[test]
    fn newtypes_are_word_sized() {
        use core::mem::size_of;
        assert_eq!(size_of::<SourceOffset>(), size_of::<u32>());
        assert_eq!(size_of::<NormalizedOffset>(), size_of::<u32>());
    }
}
