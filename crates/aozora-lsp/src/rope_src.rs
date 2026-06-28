//! [`RopeSrc`] — a [`ropey::RopeSlice`] adapter for the parser's
//! [`aozora::SanitizedSrc`] byte-source trait (#237 Tier 2, Mechanism B).
//!
//! The incremental diagnostics-only engine is generic over its sanitized
//! byte source. The `&str` impl lives in the core crate; this is the
//! rope-backed impl, kept in `aozora-lsp` so `ropey` never enters the core
//! `aozora` dependency tree. It lets [`crate::parse_cache::ParseCache`] hold
//! its sanitized buffer as a [`ropey::Rope`] and splice it incrementally,
//! feeding the engine a zero-copy view of both the cached and edited buffers
//! instead of flattening either to a fresh `String` per keystroke.
//!
//! # Cursor
//!
//! [`aozora::SanitizedSrc::byte`] is the engine region-finder's hot probe and
//! must be amortized `O(1)`. A flat `&str` indexes directly; a rope must
//! locate the chunk containing the byte. `RopeSrc` memoises the last chunk it
//! touched (its byte span plus the borrowed `&str`) in a [`Cell`]. The engine
//! scans **monotonically outward** from the edit, so consecutive probes hit
//! the same chunk and cross a chunk boundary at most once per chunk —
//! amortized `O(1)`. The 3-byte blank-line-boundary window the finder reads can
//! straddle a chunk boundary and force a constant number of re-fetches per
//! probed boundary, which does not change the amortized bound.
//!
//! `RopeSlice::chunk_at_byte` returns a `&'a str` borrowing the underlying rope
//! (lifetime `'a`, outliving the `&self` of the probe), so the memo can stash
//! it. The cursor is interior mutability over a shared `&self`, so `RopeSrc` is
//! `!Sync`; that is sound because it is built transiently under the parse mutex,
//! used for one splice, and dropped — never stored or shared across threads (the
//! `DiagBase` that *is* stored holds a `Rope`, which is `Send + Sync`).

use std::borrow::Cow;
use std::cell::Cell;
use std::fmt;
use std::ops::Range;

use aozora::SanitizedSrc;
use ropey::RopeSlice;

/// A [`ropey::RopeSlice`] viewed as the parser's [`SanitizedSrc`] byte source.
///
/// Construct with [`RopeSrc::new`] over a full-rope slice (`rope.byte_slice(..)`)
/// so byte offsets line up with the rope's own coordinates. See the module docs
/// for the cursor and `!Sync` rationale.
#[derive(Clone)]
pub struct RopeSrc<'a> {
    slice: RopeSlice<'a>,
    /// Last chunk touched by [`SanitizedSrc::byte`]: `(chunk_start, chunk_end,
    /// chunk)` where `chunk_start..chunk_end` is the chunk's byte span in
    /// `slice` and `chunk` is the chunk text. `Copy`, so the [`Cell`] is a plain
    /// interior-mutable cursor with no allocation.
    memo: Cell<Option<(usize, usize, &'a str)>>,
}

impl<'a> RopeSrc<'a> {
    /// Wrap a rope slice. Pass `rope.byte_slice(..)` so the source's byte
    /// offsets equal the rope's.
    #[must_use]
    pub fn new(slice: RopeSlice<'a>) -> Self {
        Self {
            slice,
            memo: Cell::new(None),
        }
    }
}

impl fmt::Debug for RopeSrc<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The cursor is an implementation detail; show only the byte length so
        // a failing proptest prints something stable and small.
        f.debug_struct("RopeSrc")
            .field("len_bytes", &self.slice.len_bytes())
            .finish_non_exhaustive()
    }
}

impl SanitizedSrc for RopeSrc<'_> {
    fn len(&self) -> usize {
        self.slice.len_bytes()
    }

    fn byte(&self, i: usize) -> u8 {
        if let Some((start, end, chunk)) = self.memo.get()
            && start <= i
            && i < end
        {
            return chunk.as_bytes()[i - start];
        }
        // Miss: fetch the chunk containing byte `i`. The returned `&str` borrows
        // the rope for `'a` (longer than this `&self`), so it can be stashed in
        // the cursor for the next monotone probe.
        let (chunk, chunk_start, ..) = self.slice.chunk_at_byte(i);
        self.memo
            .set(Some((chunk_start, chunk_start + chunk.len(), chunk)));
        chunk.as_bytes()[i - chunk_start]
    }

    fn slice(&self, range: Range<usize>) -> Option<Cow<'_, str>> {
        // `str::get` semantics: `None` off-bounds or on a non-char-boundary
        // range. `Cow::from(RopeSlice)` borrows a single chunk (`as_str`) and
        // only allocates when the range straddles a chunk boundary.
        self.slice.get_byte_slice(range).map(Cow::from)
    }

    #[cfg(debug_assertions)]
    fn debug_assert_unchanged_outside(
        &self,
        new: &Self,
        edit_old: Range<usize>,
        new_edit_end: usize,
    ) {
        // No-alloc restatement of the splice precondition: every byte outside
        // `edit_old` is identical between the cached buffer (`self`) and the
        // edited one (`new`). `RopeSlice == RopeSlice` walks the trees without
        // materialising a `String`, so the rope-backed proptests cross-check the
        // engine prologue in debug at no allocation cost.
        debug_assert!(
            self.slice.byte_slice(..edit_old.start) == new.slice.byte_slice(..edit_old.start)
                && self.slice.byte_slice(edit_old.end..) == new.slice.byte_slice(new_edit_end..),
            "incremental edit changed bytes outside edit_old (rope source): the \
             splice precondition was violated",
        );
    }
}
