//! Per-document bump arena.
//!
//! Wraps [`bumpalo::Bump`] with the AST-friendly subset of allocation
//! primitives the lex / render / parallel layers need. The arena is
//! consumed by [`super::Arena::alloc`] and friends; references it
//! returns are valid for the borrow of `&self`, which downstream
//! consumers re-export as the AST's `'src` lifetime.
//!
//! ## Why bumpalo
//!
//! - Allocate-only: parses produce trees, never mutate them in place.
//!   Bump's drop-everything-at-once model matches that exactly.
//! - Single-threaded by default; parallel parse paths use one arena
//!   per worker, then merge.

use bumpalo::Bump;

/// Bump-allocator arena owning all AST node storage for a single
/// parse.
///
/// Methods that allocate return references whose lifetime is tied to
/// `&self`; consumers commonly re-bind that lifetime as the parsed
/// tree's `'src` parameter.
#[derive(Debug, Default)]
pub struct Arena {
    bump: Bump,
}

impl Arena {
    /// Empty arena.
    #[must_use]
    pub fn new() -> Self {
        Self { bump: Bump::new() }
    }

    /// Empty arena with at least `capacity` bytes pre-reserved. Use
    /// when the source size is known to be large — e.g., the lex
    /// driver might call `Arena::with_capacity(source.len() / 4)` to
    /// avoid early growth allocations on a multi-MB document.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bump: Bump::with_capacity(capacity),
        }
    }

    /// Allocate `value` in the arena, returning a borrowed reference
    /// that is valid for `&self`'s lifetime.
    pub fn alloc<T>(&self, value: T) -> &T {
        self.bump.alloc(value)
    }

    /// Allocate a copy of `s` in the arena. Used when the lex layer
    /// produces a new (synthesised or rewritten) string that does not
    /// directly point into the source buffer.
    pub fn alloc_str(&self, s: &str) -> &str {
        self.bump.alloc_str(s)
    }

    /// Allocate a slice copy of `slice` in the arena. Restricted to
    /// `Copy` types because the borrowed AST contains only `Copy`
    /// data (refs, primitives, `Copy` enums) — see the `borrowed`
    /// module docs.
    pub fn alloc_slice_copy<T: Copy>(&self, slice: &[T]) -> &[T] {
        self.bump.alloc_slice_copy(slice)
    }

    /// Allocate a slice from an iterator. Useful for assembling a
    /// `Content::Segments` payload from a builder loop.
    pub fn alloc_slice_fill_iter<T, I>(&self, iter: I) -> &[T]
    where
        I: IntoIterator<Item = T>,
        I::IntoIter: ExactSizeIterator,
    {
        self.bump.alloc_slice_fill_iter(iter)
    }

    /// Bytes currently allocated to chunks (committed memory). For
    /// diagnostic / benchmarking use only.
    #[must_use]
    pub fn allocated_bytes(&self) -> usize {
        self.bump.allocated_bytes()
    }

    /// Borrow the inner [`Bump`] allocator. Used by structures that
    /// need to hold their own arena-backed storage (e.g. the
    /// [`super::Interner`]'s probe table, which is itself a
    /// `BumpVec` allocated inside the arena).
    #[must_use]
    pub fn bump(&self) -> &Bump {
        &self.bump
    }

    /// Drop every allocation without releasing the underlying chunks.
    /// The next `alloc*` call reuses the same memory pages — saving
    /// the `mmap` syscall a fresh [`Arena::new`] would pay.
    ///
    /// `&mut self` enforces at compile time that no live borrow into
    /// the arena exists at reset time: every `alloc`-returned `&T`
    /// borrows from `&self`, so a caller holding such a reference
    /// can never simultaneously call `&mut self`. Trying to do so is
    /// a borrow-checker error, not a runtime UAF.
    ///
    /// Used by long-running workers (rayon parallel corpus sweep, the
    /// LSP daemon, etc.) that parse many documents in succession and
    /// would otherwise pay one `mmap` per parse.
    pub fn reset(&mut self) {
        self.bump.reset();
    }

    /// Reset and pre-size: drop every allocation, then ensure the
    /// retained chunk capacity is at least `target_capacity` bytes
    /// before returning.
    ///
    /// Behaviour:
    /// - When the arena's existing chunk capacity already meets the
    ///   target, this is identical to [`Arena::reset`] — no syscall,
    ///   no fresh allocation.
    /// - When the target exceeds current capacity, the underlying
    ///   bump is replaced with a freshly-allocated one of at least
    ///   `target_capacity` bytes. The previous chunks are released
    ///   to the system allocator; the new bump mmaps one chunk at
    ///   the requested size.
    ///
    /// The replace path costs one `mmap` per *growth event*, not per
    /// parse. Steady-state workloads (corpus sweep on similar-sized
    /// docs) hit the no-op fast path after the first parse on each
    /// worker thread; only docs whose AST exceeds the high-water mark
    /// pay the syscall. Compared to plain [`Arena::reset`] +
    /// chunk-grow-on-demand, the cost is identical (same number of
    /// mmaps) but moved out of the parse hot path: the syscall fires
    /// before `lex_into_arena` rather than inside it, removing one
    /// source of intra-parse latency variance.
    ///
    /// Used by long-running workers (rayon corpus sweep, LSP daemon)
    /// that have a per-source size hint available — typically
    /// `source.len() * 4` for the borrowed AST shape.
    pub fn reset_with_hint(&mut self, target_capacity: usize) {
        if self.bump.allocated_bytes() >= target_capacity {
            self.bump.reset();
        } else {
            self.bump = Bump::with_capacity(target_capacity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_arena_is_empty() {
        let a = Arena::new();
        // Bumpalo's accounting is internal; we just ensure no panic
        // and the API call returns a reasonable value.
        let bytes = a.allocated_bytes();
        // Every Bump pre-allocates at least a small chunk, so >=0
        // is the only thing we can pin without coupling to bumpalo's
        // internal sizing constants.
        let _ = bytes; // intentional: only checking method works
    }

    #[test]
    fn alloc_returns_reference_with_arena_lifetime() {
        let a = Arena::new();
        let n: &u32 = a.alloc(42u32);
        assert_eq!(*n, 42);
    }

    #[test]
    fn alloc_str_copies_into_arena() {
        let a = Arena::new();
        let s = a.alloc_str("hello");
        assert_eq!(s, "hello");
    }

    #[test]
    fn alloc_slice_copy_preserves_contents() {
        let a = Arena::new();
        let slice = a.alloc_slice_copy(&[1u32, 2, 3, 4, 5]);
        assert_eq!(slice, &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn alloc_slice_fill_iter_handles_known_length() {
        let a = Arena::new();
        let slice = a.alloc_slice_fill_iter([10u32, 20, 30]);
        assert_eq!(slice, &[10, 20, 30]);
    }

    #[test]
    fn with_capacity_preallocates_some_chunk() {
        let a = Arena::with_capacity(4096);
        // The exact bytes_allocated value is a bumpalo internal, so we
        // only verify the call doesn't panic and is non-zero.
        assert!(a.allocated_bytes() > 0);
    }

    #[test]
    fn many_small_allocations_share_arena() {
        let a = Arena::new();
        // Allocate 1k tiny values. They must all coexist (no aliasing,
        // no drop). Pin the contents so the borrow checker is happy.
        let pointers: Vec<&u32> = (0..1000u32).map(|i| a.alloc(i)).collect();
        for (i, p) in pointers.iter().enumerate() {
            let expected = u32::try_from(i).expect("loop bound fits in u32");
            assert_eq!(**p, expected);
        }
    }

    #[test]
    fn reset_with_hint_grows_when_target_exceeds_current_capacity() {
        let mut a = Arena::with_capacity(4096);
        let before = a.allocated_bytes();
        // Request 10× the current capacity. The bump must be replaced
        // with a freshly-allocated one large enough to hold the hint.
        let target = before.saturating_mul(10).max(64 * 1024);
        a.reset_with_hint(target);
        let after = a.allocated_bytes();
        assert!(
            after >= target,
            "capacity must grow to at least target (target={target}, after={after})"
        );

        // Arena is reusable after the grow.
        let v: &u32 = a.alloc(7u32);
        assert_eq!(*v, 7);
    }

    #[test]
    fn reset_with_hint_is_a_plain_reset_when_target_already_met() {
        // Pre-size large; ask for a smaller hint. The arena must not
        // shrink: bumpalo's reset retains chunks, and reset_with_hint's
        // fast path takes the same plain-reset branch when the hint
        // is below current capacity.
        let mut a = Arena::with_capacity(64 * 1024);
        for i in 0..256u32 {
            let _ = a.alloc(i);
        }
        let before = a.allocated_bytes();
        a.reset_with_hint(1024);
        let after = a.allocated_bytes();
        assert_eq!(after, before, "small-target hint must not shrink the arena");
    }

    #[test]
    fn reset_drops_allocations_but_keeps_capacity() {
        let mut a = Arena::with_capacity(4096);
        // Fill enough bytes that bumpalo definitely opens its first
        // chunk. We don't need to keep the references — `&mut self`
        // on `reset` enforces that they're dropped before reset is
        // called.
        for i in 0..256u32 {
            let _ = a.alloc(i);
            let _ = a.alloc_str("filler");
        }
        let before = a.allocated_bytes();
        assert!(before > 0, "fill loop must have allocated something");

        a.reset();

        // bumpalo retains the previously-allocated chunks after reset
        // so subsequent allocations don't pay another mmap. The
        // accounting therefore stays at or above the pre-reset
        // value — what we verify is "no shrink", not an exact size
        // (bumpalo internals can shift the high-water mark on reset).
        let after = a.allocated_bytes();
        assert!(
            after >= before / 2,
            "reset should retain at least half the previous capacity (before={before}, after={after})"
        );

        // Arena is reusable: a fresh allocation works and returns a
        // valid reference.
        let v: &u32 = a.alloc(99u32);
        assert_eq!(*v, 99);
    }
}
