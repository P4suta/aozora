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

#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;

    /// 10 000 ASCII bytes (`a`..=`z`, cycling). Comfortably larger than ropey's
    /// ~1 KiB leaf, so the rope holds many chunks and a mid-buffer byte lands in
    /// an *interior* chunk whose span satisfies `0 < start < end < len`.
    fn big_string() -> String {
        (0..10_000)
            .map(|i| char::from(b'a' + u8::try_from(i % 26).unwrap()))
            .collect()
    }

    /// The byte span `[start, end)` of the chunk covering the middle of `full`,
    /// with the interior invariant the memo tests depend on asserted up front so
    /// a ropey layout change fails loudly instead of silently no-op-ing a test.
    fn middle_chunk_span(full: RopeSlice<'_>) -> (usize, usize) {
        let len = full.len_bytes();
        let (chunk, start, ..) = full.chunk_at_byte(len / 2);
        let end = start + chunk.len();
        assert!(
            0 < start && end < len,
            "fixture must span >= 3 chunks so the middle chunk is interior \
             (start={start}, end={end}, len={len})",
        );
        (start, end)
    }

    /// `Debug` names the struct and prints the byte length. Kills the
    /// `fmt -> Ok(Default::default())` mutant, which would emit nothing.
    #[test]
    fn debug_shows_struct_name_and_byte_len() {
        let rope = Rope::from_str("hello"); // 5 bytes
        let src = RopeSrc::new(rope.byte_slice(..));
        let out = format!("{src:?}");
        assert!(
            out.contains("RopeSrc"),
            "Debug must name the struct, got {out:?}",
        );
        assert!(
            out.contains("len_bytes: 5"),
            "Debug must print the byte length, got {out:?}",
        );
    }

    /// Miss then hit inside one interior chunk return the exact source bytes.
    /// Because the chunk starts at `start > 0`, the miss path (`i - chunk_start`,
    /// mutated to `i + chunk_start`) and the memo-hit path (`i - start`, mutated
    /// to `i + start`) both index far past the chunk and panic under mutation.
    #[test]
    fn byte_miss_then_hit_return_exact_bytes() {
        let s = big_string();
        let rope = Rope::from_str(&s);
        let (start, end) = middle_chunk_span(rope.byte_slice(..));
        let src = RopeSrc::new(rope.byte_slice(..));

        // First probe: memo empty -> miss path (line 94).
        assert_eq!(
            src.byte(start),
            s.as_bytes()[start],
            "miss-path byte at the interior chunk start must equal the source",
        );
        // Second probe in the same chunk: memo hit (line 86).
        let hit = start + 1;
        assert!(
            hit < end,
            "the second probe must stay inside the memo chunk"
        );
        assert_eq!(
            src.byte(hit),
            s.as_bytes()[hit],
            "memo-hit byte must equal the source byte",
        );
    }

    /// A byte just *below* the memoised chunk must refetch, not reuse the memo.
    /// Kills `start <= i` -> `start > i`: the mutated guard reuses the chunk and
    /// evaluates `chunk[i - start]` with `i < start`, indexing out of bounds.
    #[test]
    fn byte_below_memoised_chunk_refetches() {
        let s = big_string();
        let rope = Rope::from_str(&s);
        let (start, _end) = middle_chunk_span(rope.byte_slice(..));
        let src = RopeSrc::new(rope.byte_slice(..));
        src.byte(start); // memoise the interior chunk [start, end)

        let below = start - 1;
        assert_eq!(
            src.byte(below),
            s.as_bytes()[below],
            "a probe below the memo chunk must refetch, not reuse it",
        );
    }

    /// The exclusive chunk-end byte (first byte of the *next* chunk) must
    /// refetch. Unmutated: `end < end` is false. Kills three mutants at once:
    /// `<` -> `==` and `<` -> `<=` both make the guard accept `i == end`, and
    /// `chunk_start + chunk.len()` -> `chunk_start * chunk.len()` inflates the
    /// stored end so `i == end` slips under it. Each reuses the chunk and
    /// indexes `chunk[end - start] == chunk[chunk.len()]` out of bounds.
    #[test]
    fn byte_at_chunk_end_boundary_refetches() {
        let s = big_string();
        let rope = Rope::from_str(&s);
        let (start, end) = middle_chunk_span(rope.byte_slice(..));
        let src = RopeSrc::new(rope.byte_slice(..));
        src.byte(start); // memoise [start, end)

        assert_eq!(
            src.byte(end),
            s.as_bytes()[end],
            "the exclusive chunk-end byte must come from the next chunk",
        );
    }

    /// A byte *above* the memoised chunk must refetch. Kills `i < end` ->
    /// `i > end`: the mutated guard accepts `i > end` and reuses the chunk,
    /// indexing `chunk[i - start]` past the chunk end.
    #[test]
    fn byte_above_memoised_chunk_refetches() {
        let s = big_string();
        let rope = Rope::from_str(&s);
        let (start, end) = middle_chunk_span(rope.byte_slice(..));
        let src = RopeSrc::new(rope.byte_slice(..));
        src.byte(start); // memoise [start, end)

        let above = end + 1;
        assert!(above < s.len(), "the probe must stay within the buffer");
        assert_eq!(
            src.byte(above),
            s.as_bytes()[above],
            "a probe above the memo chunk must refetch, not reuse it",
        );
    }

    /// `slice` returns exact substrings (including across a multi-byte char) and
    /// `None` for non-char-boundary and out-of-bounds ranges. Kills every
    /// `slice -> Some("")` / `Some("xyzzy")` mutant: they return a fixed string
    /// for the valid ranges and `Some` for the ranges that must be `None`.
    #[test]
    fn slice_returns_exact_substrings_and_none_off_bounds() {
        let rope = Rope::from_str("aあb"); // a(1) + あ(3) + b(1) = 5 bytes
        let src = RopeSrc::new(rope.byte_slice(..));

        assert_eq!(src.slice(0..1).as_deref(), Some("a"), "ASCII head slice");
        assert_eq!(
            src.slice(1..4).as_deref(),
            Some("あ"),
            "slice spanning the whole multi-byte char",
        );
        assert_eq!(
            src.slice(0..5).as_deref(),
            Some("aあb"),
            "full-buffer slice",
        );
        assert_eq!(
            src.slice(0..2).as_deref(),
            None,
            "a range splitting the multi-byte char yields None",
        );
        assert_eq!(
            src.slice(0..6).as_deref(),
            None,
            "an out-of-bounds range yields None",
        );
    }

    /// A splice claiming a smaller edit than actually happened must trip the
    /// debug precondition. Kills the `debug_assert_unchanged_outside -> ()`
    /// mutant, which drops the check and would not panic. `#[cfg(debug_assertions)]`
    /// mirrors the method's own gate so a release test build still compiles.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "changed bytes outside edit_old")]
    fn debug_assert_unchanged_outside_panics_on_prefix_change() {
        let cached = Rope::from_str("XYZ");
        let edited = Rope::from_str("AYZ"); // byte 0 differs
        let cached_src = RopeSrc::new(cached.byte_slice(..));
        let edited_src = RopeSrc::new(edited.byte_slice(..));
        // Claim only byte 1 changed; byte 0 differs too -> precondition violated.
        cached_src.debug_assert_unchanged_outside(&edited_src, 1..2, 2);
    }

    /// The other side of the precondition: an edit confined to `edit_old` leaves
    /// prefix and suffix identical, so the check must *not* panic.
    #[cfg(debug_assertions)]
    #[test]
    fn debug_assert_unchanged_outside_accepts_valid_splice() {
        let cached = Rope::from_str("XYZ");
        let edited = Rope::from_str("XQZ"); // only byte 1 changed
        let cached_src = RopeSrc::new(cached.byte_slice(..));
        let edited_src = RopeSrc::new(edited.byte_slice(..));
        cached_src.debug_assert_unchanged_outside(&edited_src, 1..2, 2);
    }
}
