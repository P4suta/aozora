//! Output channel abstraction for trigger-byte scanners.
//!
//! Replaces the previous `Vec<u32>` eager-allocation shape with a
//! visitor-style sink so backends can write trigger offsets directly
//! into the caller's preferred buffer (heap `Vec`, bumpalo arena
//! `BumpVec`, or a counting sink that records nothing) without the
//! per-call heap → arena memcpy that the dyn-trait API forced on
//! every parse.
//!
//! The sink trait stays generic-method on purpose: every backend's
//! scan loop monomorphises against the concrete sink type, which
//! lets the LLVM inliner fold the `push` call into the SIMD inner
//! loop with no virtual dispatch overhead. Dispatching from a
//! `&dyn TriggerScanner` is preserved by holding the legacy
//! `scan_offsets` -> `Vec<u32>` shape on the trait; the new sink
//! path lives as `inherent` methods on each backend, called either
//! through the runtime [`crate::BackendChoice`] dispatcher (added
//! in a follow-up step) or directly when the backend is statically
//! known.

use alloc::vec::Vec;

#[cfg(feature = "std")]
use bumpalo::collections::Vec as BumpVec;

/// Sink for trigger byte offsets emitted by a
/// [`crate::TriggerScanner`].
///
/// Implementations decide where each offset lives — heap `Vec`,
/// arena `BumpVec`, a count-only [`CountSink`], or a custom buffer
/// outside this crate. Scanners call [`OffsetSink::reserve`] once
/// when the upper bound is known so the sink can pre-allocate;
/// thereafter every match calls [`OffsetSink::push`].
///
/// The trait is intentionally not `dyn`-compatible: monomorphising
/// against the concrete sink type lets the SIMD inner loop inline
/// the push, which is the whole point of having a streaming sink in
/// the first place.
pub trait OffsetSink {
    /// Append one trigger byte offset to the sink.
    fn push(&mut self, offset: u32);

    /// Hint the sink that `additional` more pushes are expected.
    /// Default impl is a no-op for sinks that cannot reserve.
    #[inline]
    fn reserve(&mut self, _additional: usize) {}
}

impl OffsetSink for Vec<u32> {
    #[inline]
    fn push(&mut self, offset: u32) {
        Vec::push(self, offset);
    }

    #[inline]
    fn reserve(&mut self, additional: usize) {
        Vec::reserve(self, additional);
    }
}

#[cfg(feature = "std")]
impl OffsetSink for BumpVec<'_, u32> {
    #[inline]
    fn push(&mut self, offset: u32) {
        BumpVec::push(self, offset);
    }

    #[inline]
    fn reserve(&mut self, additional: usize) {
        BumpVec::reserve(self, additional);
    }
}

/// Sink that counts pushes without storing any offset.
///
/// Useful for pre-flight counting (capacity probes, density
/// measurement) and for benchmarks that want to isolate the scanner
/// cost from the allocator. Counts saturate at `usize::MAX`, which
/// is unreachable in practice (a `usize::MAX`-byte source already
/// exhausts the address space).
#[derive(Debug, Default, Clone, Copy)]
pub struct CountSink {
    count: usize,
}

impl CountSink {
    /// Construct a fresh counter at zero.
    #[must_use]
    pub const fn new() -> Self {
        Self { count: 0 }
    }

    /// Number of `push` calls observed since construction.
    #[must_use]
    pub const fn count(&self) -> usize {
        self.count
    }
}

impl OffsetSink for CountSink {
    #[inline]
    fn push(&mut self, _offset: u32) {
        self.count = self.count.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn vec_sink_pushes_in_order() {
        let mut sink: Vec<u32> = Vec::new();
        // UFCS to exercise the trait impl directly rather than the
        // intrinsic `Vec::reserve` method (clippy's
        // `reserve_after_initialization` lint would otherwise nudge
        // toward `Vec::with_capacity`, defeating the trait test).
        OffsetSink::reserve(&mut sink, 3);
        OffsetSink::push(&mut sink, 1);
        OffsetSink::push(&mut sink, 4);
        OffsetSink::push(&mut sink, 9);
        assert_eq!(sink, vec![1u32, 4, 9]);
    }

    #[cfg(feature = "std")]
    #[test]
    fn bumpvec_sink_pushes_in_order_into_arena() {
        let arena = Bump::new();
        let mut sink: BumpVec<'_, u32> = BumpVec::new_in(&arena);
        OffsetSink::reserve(&mut sink, 3);
        OffsetSink::push(&mut sink, 1);
        OffsetSink::push(&mut sink, 4);
        OffsetSink::push(&mut sink, 9);
        assert_eq!(sink.as_slice(), &[1u32, 4, 9]);
    }

    #[test]
    fn count_sink_only_counts() {
        let mut sink = CountSink::new();
        assert_eq!(sink.count(), 0);
        sink.push(0);
        sink.push(42);
        sink.push(u32::MAX);
        assert_eq!(sink.count(), 3);
    }

    #[test]
    fn count_sink_reserve_is_no_op() {
        let mut sink = CountSink::new();
        OffsetSink::reserve(&mut sink, 1024);
        assert_eq!(sink.count(), 0);
    }

    #[cfg(feature = "std")]
    use bumpalo::Bump;
}
