#![expect(
    clippy::expect_used,
    reason = "Span construction maintains source bounds and UTF-8 boundaries"
)]

/// Byte-range span whose parser-produced endpoints lie on UTF-8 boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct Span {
    /// Inclusive start byte offset into the source buffer.
    pub start: u32,
    /// Exclusive end byte offset.
    pub end: u32,
}

impl Span {
    /// Construct a half-open span from byte offsets.
    #[must_use]
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    /// Span length in bytes.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if `end < start`; release builds wrap.
    #[must_use]
    pub const fn len(self) -> u32 {
        self.end - self.start
    }

    /// Whether the span covers zero bytes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Slice the source buffer by this span.
    ///
    /// # Panics
    ///
    /// Panics if the span is out of bounds or its endpoints are not UTF-8 boundaries.
    #[must_use]
    pub fn slice(self, source: &str) -> &str {
        let start = self.start as usize;
        let end = self.end as usize;
        source
            .get(start..end)
            .expect("span must align to UTF-8 char boundaries in source")
    }

    /// Shift both endpoints by `by` bytes, saturating at the `u32` bounds.
    #[must_use]
    pub fn shifted(self, by: i64) -> Self {
        Self {
            start: shift_clamp(self.start, by),
            end: shift_clamp(self.end, by),
        }
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamp guarantees the value is inside the u32 range before the cast"
)]
fn shift_clamp(endpoint: u32, by: i64) -> u32 {
    by.saturating_add(i64::from(endpoint))
        .clamp(0, i64::from(u32::MAX)) as u32
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
        assert_eq!(Span::new(3, 6).slice(src), "空");
    }

    #[test]
    #[should_panic(expected = "span must align to UTF-8 char boundaries")]
    fn slice_panics_on_misaligned_boundary() {
        let src = "青空";
        let _slice: &str = Span::new(1, 4).slice(src);
    }

    #[test]
    fn shifted_translates_both_endpoints() {
        assert_eq!(Span::new(3, 7).shifted(10), Span::new(13, 17));
        assert_eq!(Span::new(13, 17).shifted(-10), Span::new(3, 7));
        assert_eq!(Span::new(3, 7).shifted(100).len(), 4);
    }

    #[test]
    fn shifted_clamps_at_zero_on_underflow() {
        assert_eq!(Span::new(2, 5).shifted(-100), Span::new(0, 0));
        assert_eq!(shift_clamp(7, -7), 0);
        assert_eq!(shift_clamp(7, -8), 0);
        assert_eq!(shift_clamp(u32::MAX, i64::MIN), 0);
    }

    #[test]
    fn shifted_clamps_at_u32_max_on_overflow() {
        assert_eq!(
            Span::new(u32::MAX - 1, u32::MAX).shifted(100),
            Span::new(u32::MAX, u32::MAX)
        );
        assert_eq!(shift_clamp(u32::MAX - 1, 1), u32::MAX);
        assert_eq!(shift_clamp(u32::MAX - 1, 2), u32::MAX);
        assert_eq!(shift_clamp(u32::MAX, i64::MAX), u32::MAX);
    }

    #[test]
    fn span_is_8_bytes_on_64_bit_target() {
        use core::mem::size_of;
        assert_eq!(size_of::<Span>(), 8);
    }
}
