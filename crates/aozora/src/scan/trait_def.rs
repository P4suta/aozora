//! Output channel abstraction for trigger-byte scanners.
//!
//! Replaces an eager `Vec<u32>`-returning entry shape with a
//! visitor-style sink so kernels can write trigger offsets directly
//! into the caller's preferred buffer (heap `Vec`, bumpalo arena
//! `BumpVec`, or a counting sink that records nothing) without the
//! heap → arena memcpy a returned `Vec<u32>` would force on every
//! parse.
//!
//! The sink trait stays generic-method on purpose: the scan loop
//! monomorphises against the concrete sink type, which lets the LLVM
//! inliner fold the `push` call into the match-emit loop with no
//! virtual dispatch overhead — so [`crate::scan::scan_offsets`] emits
//! straight into the caller's buffer with no per-match indirection.

use std::vec::Vec;

use bumpalo::collections::Vec as BumpVec;

/// Sink for trigger byte offsets emitted by the production scanner
/// ([`crate::scan::scan_offsets`]).
///
/// Implementations decide where each offset lives — heap `Vec`,
/// arena `BumpVec`, or a custom buffer outside this crate. Every match calls
/// [`OffsetSink::push`].
///
/// The trait is intentionally not `dyn`-compatible: monomorphising
/// against the concrete sink type lets the SIMD inner loop inline
/// the push, which is the whole point of having a streaming sink in
/// the first place.
pub(crate) trait OffsetSink {
    /// Append one trigger byte offset to the sink.
    fn push(&mut self, offset: u32);
}

impl OffsetSink for Vec<u32> {
    #[inline]
    fn push(&mut self, offset: u32) {
        Self::push(self, offset);
    }
}

impl OffsetSink for BumpVec<'_, u32> {
    #[inline]
    fn push(&mut self, offset: u32) {
        BumpVec::push(self, offset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;

    #[test]
    fn vec_sink_pushes_in_order() {
        let mut sink: Vec<u32> = Vec::new();
        OffsetSink::push(&mut sink, 1);
        OffsetSink::push(&mut sink, 4);
        OffsetSink::push(&mut sink, 9);
        assert_eq!(sink, vec![1u32, 4, 9]);
    }

    #[test]
    fn bumpvec_sink_pushes_in_order_into_arena() {
        let arena = Bump::new();
        let mut sink: BumpVec<'_, u32> = BumpVec::new_in(&arena);
        OffsetSink::push(&mut sink, 1);
        OffsetSink::push(&mut sink, 4);
        OffsetSink::push(&mut sink, 9);
        assert_eq!(sink.as_slice(), &[1u32, 4, 9]);
    }

    use bumpalo::Bump;
}
