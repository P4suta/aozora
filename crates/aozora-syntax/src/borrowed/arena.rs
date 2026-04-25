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
//! - Comrak (which the workspace also uses, ADR-0001) already depends
//!   on bumpalo, so adding it here adds no new transitive dependency.
//! - Single-threaded by default; the parallel parse path (Move 3 in
//!   `aozora-parallel`) uses one arena per worker, then merges.
//!
//! ## Future: interner
//!
//! Innovation I-7 of the 0.2.0 plan calls for a string interner that
//! deduplicates repeated readings (`の`, `に`, etc.) into a single
//! arena allocation. The interner is intentionally deferred to a
//! follow-up commit because (a) Move 1.4 is purely a type-shape
//! change with no runtime consumer yet — Move 2 builds the lex layer
//! that will exercise the interner — and (b) the interner can be
//! added to [`Arena`] without touching the AST type signatures.

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
}
