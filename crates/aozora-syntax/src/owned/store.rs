//! Flat backing store for the owned AST.
//!
//! The former borrowed tree leaned on a bumpalo arena for two variable-length
//! payloads: `NonEmpty<Content>` runs and `&[Segment]` slices. The owned
//! store replaces both with half-open ranges ([`ContentRange`] / [`SegRange`])
//! into flat `Vec`s held by [`NodeStore`], alongside the [`StrInterner`] that
//! owns every interned string. `StrId` / range payloads on the owned nodes
//! resolve against this store.

use super::intern::{StrId, StrInterner};
use super::payload::{ContentOwned, SegmentOwned};

/// Half-open run of [`ContentOwned`] in [`NodeStore::resolve_content_range`];
/// `len >= 1` for runs mapped from a borrowed `NonEmpty<Content>`. Borrowed
/// `NonEmpty<Content<'src>>` -> this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentRange {
    /// Index of the first [`ContentOwned`] in the store's content `Vec`.
    pub start: u32,
    /// Number of [`ContentOwned`] entries in the run.
    pub len: u32,
}

/// Half-open run of [`SegmentOwned`] in [`NodeStore::resolve_seg_range`].
/// Borrowed `&'src [Segment<'src>]` -> this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegRange {
    /// Index of the first [`SegmentOwned`] in the store's segment `Vec`.
    pub start: u32,
    /// Number of [`SegmentOwned`] entries in the run.
    pub len: u32,
}

/// Owned backing store: the string interner plus the flat content / segment
/// `Vec`s the owned nodes' [`StrId`] / range payloads resolve against.
///
/// Owns what the bumpalo arena owned in the borrowed pipeline. Not `Copy`
/// (owns heap storage); not `PartialEq` (the interner's `InternStats` field
/// is not `PartialEq`).
#[derive(Debug, Clone, Default)]
pub struct NodeStore {
    /// String interner backing every [`StrId`] in the tree.
    pub interner: StrInterner,
    /// Flat pool of [`ContentOwned`] entries; [`ContentRange`]s index here.
    contents: Vec<ContentOwned>,
    /// Flat pool of [`SegmentOwned`] entries; [`SegRange`]s index here.
    segments: Vec<SegmentOwned>,
}

impl NodeStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern `s` into the store's interner, returning a stable [`StrId`].
    pub fn intern(&mut self, s: &str) -> StrId {
        self.interner.intern(s)
    }

    /// Resolve a [`StrId`] to its interned bytes.
    ///
    /// # Panics
    ///
    /// Panics if `id` was not produced by this store's interner.
    #[must_use]
    pub fn resolve_str(&self, id: StrId) -> &str {
        self.interner.resolve(id)
    }

    /// Append a content run and return the [`ContentRange`] that addresses it.
    ///
    /// # Panics
    ///
    /// Panics if the content pool would exceed `u32::MAX` entries — not
    /// reachable for any realistic document.
    pub fn push_contents(&mut self, items: &[ContentOwned]) -> ContentRange {
        let start =
            u32::try_from(self.contents.len()).expect("content pool exceeds u32 entry count");
        let len = u32::try_from(items.len()).expect("content run exceeds u32 length");
        self.contents.extend_from_slice(items);
        ContentRange { start, len }
    }

    /// Append a segment run and return the [`SegRange`] that addresses it.
    ///
    /// # Panics
    ///
    /// Panics if the segment pool would exceed `u32::MAX` entries — not
    /// reachable for any realistic document.
    pub fn push_segments(&mut self, items: &[SegmentOwned]) -> SegRange {
        let start =
            u32::try_from(self.segments.len()).expect("segment pool exceeds u32 entry count");
        let len = u32::try_from(items.len()).expect("segment run exceeds u32 length");
        self.segments.extend_from_slice(items);
        SegRange { start, len }
    }

    /// Resolve a [`ContentRange`] to its sub-slice of the content pool.
    ///
    /// # Panics
    ///
    /// Panics if the range falls outside the content pool.
    #[must_use]
    pub fn resolve_content_range(&self, range: ContentRange) -> &[ContentOwned] {
        let start = range.start as usize;
        &self.contents[start..start + range.len as usize]
    }

    /// Resolve a [`SegRange`] to its sub-slice of the segment pool.
    ///
    /// # Panics
    ///
    /// Panics if the range falls outside the segment pool.
    #[must_use]
    pub fn resolve_seg_range(&self, range: SegRange) -> &[SegmentOwned] {
        let start = range.start as usize;
        &self.segments[start..start + range.len as usize]
    }

    /// Owned mirror of `borrowed::Content::as_plain`
    /// over a length-1 content run: `Some(text)` iff the run is exactly one
    /// [`ContentOwned::Plain`]; `None` for a `Segments` run or any `len != 1`.
    ///
    /// Consumers that walked the borrowed `NonEmpty<Content>` payload fields
    /// (ruby base/reading, forward-format target, …) with `.get().as_plain()`
    /// read the owned [`ContentRange`] through this.
    ///
    /// # Panics
    ///
    /// Panics if `range` falls outside the content pool (via
    /// [`Self::resolve_content_range`]).
    #[must_use]
    pub fn content_range_as_plain(&self, range: ContentRange) -> Option<&str> {
        match self.resolve_content_range(range) {
            [ContentOwned::Plain(id)] => Some(self.resolve_str(*id)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // UNIT TEST 4: ranges resolve to the correct sub-slices.
    #[test]
    fn ranges_resolve_to_correct_sub_slices() {
        let mut store = NodeStore::new();
        let a = store.intern("a");
        let b = store.intern("b");
        let c = store.intern("c");

        // Two separate content runs must not alias.
        let first = store.push_contents(&[ContentOwned::Plain(a)]);
        let second = store.push_contents(&[ContentOwned::Plain(b), ContentOwned::Plain(c)]);

        assert_eq!(first, ContentRange { start: 0, len: 1 });
        assert_eq!(second, ContentRange { start: 1, len: 2 });
        assert_eq!(
            store.resolve_content_range(first),
            &[ContentOwned::Plain(a)]
        );
        assert_eq!(
            store.resolve_content_range(second),
            &[ContentOwned::Plain(b), ContentOwned::Plain(c)]
        );

        // Segment ranges resolve independently.
        let seg = store.push_segments(&[SegmentOwned::Text(a), SegmentOwned::Text(c)]);
        assert_eq!(seg, SegRange { start: 0, len: 2 });
        assert_eq!(
            store.resolve_seg_range(seg),
            &[SegmentOwned::Text(a), SegmentOwned::Text(c)]
        );
    }

    #[test]
    fn content_range_as_plain_mirrors_borrowed() {
        let mut store = NodeStore::new();
        let a = store.intern("foo");
        let b = store.intern("bar");

        // Length-1 `Plain` run → the interned text (the 99%+ majority case).
        let plain = store.push_contents(&[ContentOwned::Plain(a)]);
        assert_eq!(store.content_range_as_plain(plain), Some("foo"));

        // A `Segments` run → `None` (mixed content, like borrowed `as_plain`).
        let seg = store.push_segments(&[SegmentOwned::Text(a)]);
        let mixed = store.push_contents(&[ContentOwned::Segments(seg)]);
        assert_eq!(store.content_range_as_plain(mixed), None);

        // A multi-entry run → `None` (only the length-1 fast path is plain).
        let two = store.push_contents(&[ContentOwned::Plain(a), ContentOwned::Plain(b)]);
        assert_eq!(store.content_range_as_plain(two), None);
    }
}
