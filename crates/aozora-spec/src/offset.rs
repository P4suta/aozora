//! Coordinate-system newtypes for byte offsets.
//!
//! The lex pipeline operates over **three** distinct coordinate
//! spaces that all happen to use `u32` byte offsets:
//!
//! 1. **Source** — bytes of the original input string the caller
//!    handed to [`Document::new`](`crate`)<!-- doc-link via meta crate -->.
//!    BOM, CRLF and decorative-rule positions are still in their
//!    original positions.
//! 2. **Sanitized source** — bytes of the Phase 0 output.
//!    BOM-stripped, CR/LF-normalised, accent-decomposed,
//!    decorative-rule-isolated. For the typical document
//!    (no BOM, only LF, no `〔...〕` accent spans, no long
//!    decorative rule lines) sanitized == source byte-for-byte;
//!    [`Span`](crate::Span) values are quoted in this coordinate
//!    space throughout the public API.
//! 3. **Normalized** — bytes of the PUA-sentinel-rewritten text
//!    that the placeholder registry indexes. Each Aozora construct
//!    occupies one PUA codepoint here regardless of its source
//!    width.
//!
//! Pre-Phase-E2 every offset was a bare `u32` and the compiler had no
//! way to catch a "I passed a normalized offset where the API
//! expected a source offset" mistake. The newtypes [`SourceOffset`]
//! and [`NormalizedOffset`] make those mismatches a build error
//! without paying any runtime cost — both compile to a `u32` field
//! access.
//!
//! # Conversion policy
//!
//! - `u32 → SourceOffset` and `u32 → NormalizedOffset` are infallible
//!   (`From` impls). Constructing a coordinate from a raw byte index
//!   is the caller acting as the authority on which space the index
//!   names.
//! - There is **no** cross-conversion between [`SourceOffset`] and
//!   [`NormalizedOffset`]. Translating between coordinate spaces
//!   requires a side-table (the `source_nodes` slice on
//!   [`BorrowedLexOutput`]), and the lack of `From` impls forces
//!   callers to go through that table rather than reinterpret a
//!   `u32` they already have.
//! - Both newtypes expose a [`get`](SourceOffset::get) accessor for
//!   internal arithmetic.

/// Byte offset into the **sanitized source** text (Phase 0 output).
///
/// For the typical document where Phase 0 is the identity (no BOM,
/// only LF, no `〔...〕` accent spans, no long decorative rule lines)
/// the sanitized-source coordinate space coincides with the original
/// source the caller passed in.
///
/// Editor surfaces and LSP-style queries that hold a byte offset into
/// a buffer the user is editing produce values in this space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceOffset(pub u32);

impl SourceOffset {
    /// Construct from a raw byte index. The caller is asserting that
    /// `v` names a position in the sanitized-source coordinate space.
    #[must_use]
    pub const fn new(v: u32) -> Self {
        Self(v)
    }

    /// Underlying byte index. Useful for arithmetic and comparison
    /// against existing `u32` data.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for SourceOffset {
    fn from(v: u32) -> Self {
        Self(v)
    }
}

/// Byte offset into the **normalized text** the lex pipeline emits
/// for the downstream CommonMark parser.
///
/// The normalized text replaces every recognised Aozora construct
/// with one of the four PUA sentinel codepoints
/// ([`crate::Sentinel`]); the placeholder registry maps each sentinel
/// position back to its originating construct. Renderers that walk
/// the normalized text and dispatch on sentinel hits operate in this
/// coordinate space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NormalizedOffset(pub u32);

impl NormalizedOffset {
    /// Construct from a raw byte index. The caller is asserting that
    /// `v` names a position in the normalized-text coordinate space.
    #[must_use]
    pub const fn new(v: u32) -> Self {
        Self(v)
    }

    /// Underlying byte index. Useful for arithmetic and comparison
    /// against existing `u32` data.
    #[must_use]
    pub const fn get(self) -> u32 {
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

    /// The lack of cross-conversion is load-bearing — the pipeline
    /// distinguishes source offsets from normalized offsets at the
    /// type level. This test pins the absence of `From<SourceOffset>
    /// for NormalizedOffset` (and vice versa) by attempting the
    /// conversion through `into()` would fail to compile if the impl
    /// were ever added accidentally. We can't write a "must-not-
    /// compile" test in plain rustc, so we instead document the
    /// intent here.
    #[test]
    fn coordinate_spaces_are_disjoint() {
        // The newtypes share an underlying `u32` representation but
        // are distinct types. Callers translate between them via the
        // `BorrowedLexOutput::source_nodes` side-table, never by
        // direct casting.
        let s = SourceOffset::new(5);
        let n = NormalizedOffset::new(5);
        // Both reach the same underlying byte value — but reaching
        // it goes through `.get()` in either direction, signalling
        // the cross-space hop explicitly.
        assert_eq!(s.get(), n.get());
    }

    #[test]
    fn newtypes_are_word_sized() {
        // Pin the no-overhead promise: each newtype is exactly the
        // size of a `u32`. A future change that wraps the inner
        // value in something heavier (e.g. `NonZeroU32`, an enum)
        // surfaces here.
        use core::mem::size_of;
        assert_eq!(size_of::<SourceOffset>(), size_of::<u32>());
        assert_eq!(size_of::<NormalizedOffset>(), size_of::<u32>());
    }
}
