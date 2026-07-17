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
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct Span {
    /// Inclusive start byte offset into the source buffer.
    pub start: u32,
    /// Exclusive end byte offset. Always `>= start` for parser-produced
    /// spans, so [`len`](Self::len) does not underflow.
    pub end: u32,
}

impl Span {
    /// Construct a span from its `start` (inclusive) and `end`
    /// (exclusive) byte offsets. Callers are responsible for passing
    /// `end >= start`; the parser always does.
    #[must_use]
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// Length of the span in bytes (`end - start`).
    ///
    /// # Panics
    ///
    /// Underflows (panicking in debug, wrapping in release) if
    /// `end < start`. Parser-produced spans never violate this.
    #[must_use]
    pub const fn len(self) -> u32 {
        self.end - self.start
    }

    /// Whether the span covers zero bytes (`start == end`). An empty
    /// span still carries a position — e.g. a marker inserted between
    /// two characters.
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

    /// Translate both endpoints by `by` bytes, saturating at the `u32`
    /// range. A uniform shift preserves `end >= start`, so [`len`](Self::len)
    /// stays well-defined.
    ///
    /// Used by the incremental re-parse engine ([`aozora`'s segment
    /// cache](https://p4suta.github.io/aozora/), #237) to rebase a span
    /// produced by lexing a document *segment* (segment-local coordinates)
    /// back into whole-document coordinates by adding the segment's start
    /// offset. `by` is `i64` because a rebase can move a span either way;
    /// the clamp keeps a negative shift from underflowing the `u32`
    /// endpoints.
    #[must_use]
    pub const fn shifted(self, by: i64) -> Self {
        Self {
            start: shift_clamp(self.start, by),
            end: shift_clamp(self.end, by),
        }
    }
}

/// Add `by` to `endpoint`, saturating into the `u32` range. `const`-safe
/// (no `Ord::clamp`, which is not yet const-stable).
///
/// Mutation note: the two boundary comparisons carry a pair of *equivalent*
/// mutants that no test can distinguish, so they are documented here rather
/// than chased. `<` → `<=` on the lower guard differs only at `shifted == 0`,
/// where both the `<= 0` branch (`0`) and the fall-through (`0 as u32`) yield
/// `0`; `>` → `>=` on the upper guard differs only at `shifted == u32::MAX`,
/// where both the `>=` branch and the fall-through (`u32::MAX as u32`) yield
/// `u32::MAX`. The killable `>` → `==` variant (which would truncate instead of
/// saturating past the top of the range) is pinned by
/// `tests::shifted_clamps_at_u32_max_on_overflow`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the bounds check guarantees 0 <= shifted <= u32::MAX before the cast"
)]
const fn shift_clamp(endpoint: u32, by: i64) -> u32 {
    let shifted = endpoint as i64 + by;
    if shifted < 0 {
        0
    } else if shifted > u32::MAX as i64 {
        u32::MAX
    } else {
        shifted as u32
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
    fn shifted_translates_both_endpoints() {
        assert_eq!(Span::new(3, 7).shifted(10), Span::new(13, 17));
        assert_eq!(Span::new(13, 17).shifted(-10), Span::new(3, 7));
        // length is preserved under a uniform shift
        assert_eq!(Span::new(3, 7).shifted(100).len(), 4);
    }

    #[test]
    fn shifted_clamps_at_zero_on_underflow() {
        // A negative shift larger than `start` clamps both endpoints at 0
        // rather than wrapping the `u32`.
        assert_eq!(Span::new(2, 5).shifted(-100), Span::new(0, 0));
    }

    #[test]
    fn shifted_clamps_at_u32_max_on_overflow() {
        // A positive shift that pushes an endpoint past `u32::MAX`
        // saturates at the top of the range rather than truncating the
        // `i64` sum back into `u32` (which would wrap to a small value).
        assert_eq!(
            Span::new(u32::MAX - 1, u32::MAX).shifted(100),
            Span::new(u32::MAX, u32::MAX)
        );
    }

    #[test]
    fn span_is_8_bytes_on_64_bit_target() {
        // The whole point of u32 endpoints (vs usize) is the size win
        // on 64-bit targets; pin it so a future change has to think.
        use core::mem::size_of;
        assert_eq!(size_of::<Span>(), 8);
    }
}
