//! Byte-range span over a UTF-8 source buffer.
//!
//! `u32` (rather than `usize`) caps the addressable source at 4 GiB,
//! which is roughly 4 000× the largest plausible Aozora Bunko work — and
//! halves span size on 64-bit targets, which compounds across the
//! thousands of nodes a long novel produces.

/// Byte-range span. Both endpoints are guaranteed to fall on UTF-8
/// character boundaries when produced by the parser; callers can
/// safely slice the source with them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    #[must_use]
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    #[must_use]
    pub const fn len(self) -> u32 {
        self.end - self.start
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Slice the source buffer by this span. Assumes `self` was produced
    /// by the parser and therefore sits on UTF-8 boundaries.
    ///
    /// # Panics
    ///
    /// Panics if `self` does not align to UTF-8 char boundaries in
    /// `source`. Parser-produced spans always do; a panic here signals
    /// a bug upstream.
    #[must_use]
    pub fn slice(self, source: &str) -> &str {
        let start = self.start as usize;
        let end = self.end as usize;
        source
            .get(start..end)
            .expect("span must align to UTF-8 char boundaries in source")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_records_endpoints() {
        let s = Span::new(3, 7);
        assert_eq!(s.start, 3);
        assert_eq!(s.end, 7);
    }

    #[test]
    fn len_is_end_minus_start() {
        assert_eq!(Span::new(2, 5).len(), 3);
        assert_eq!(Span::new(0, 0).len(), 0);
    }

    #[test]
    fn empty_span_reports_empty() {
        assert!(Span::new(4, 4).is_empty());
        assert!(!Span::new(4, 5).is_empty());
    }

    #[test]
    fn slice_extracts_exact_byte_range() {
        let src = "hello, world";
        assert_eq!(Span::new(7, 12).slice(src), "world");
        assert_eq!(Span::new(0, 5).slice(src), "hello");
    }

    #[test]
    fn slice_works_at_utf8_boundary() {
        let src = "青空文庫";
        // Each kanji is 3 bytes UTF-8.
        assert_eq!(Span::new(3, 6).slice(src), "空");
    }

    #[test]
    #[should_panic(expected = "span must align to UTF-8 char boundaries")]
    fn slice_panics_on_misaligned_boundary() {
        let src = "青空"; // 6 bytes total, 0..3 = 青, 3..6 = 空
        // The slice is `#[must_use]` but the body is the panic, not
        // the return value — assign to a typed binding to consume it.
        let _slice: &str = Span::new(1, 4).slice(src);
    }

    #[test]
    fn span_is_8_bytes_on_64_bit_target() {
        // The whole point of u32 endpoints (vs usize) is the size win
        // on 64-bit targets; pin it so a future change has to think.
        use core::mem::size_of;
        assert_eq!(size_of::<Span>(), 8);
    }
}
