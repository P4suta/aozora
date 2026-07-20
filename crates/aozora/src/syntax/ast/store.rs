//! Flat backing store for the owned AST.
//!
//! Two variable-length payloads — `NonEmpty<Content>` runs and `[Segment]`
//! slices — are stored as half-open ranges ([`ContentRange`] / [`SegRange`])
//! into flat `Vec`s held by [`NodeStore`], alongside the [`StrInterner`] that
//! owns every interned string. `StrId` / range payloads on the owned nodes
//! resolve against this store.

use super::intern::{StrId, StrInterner};
use super::payload::{Content, Segment};
use std::sync::Arc;

/// Opaque non-empty run of [`Content`] owned by a [`crate::Snapshot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ContentRange {
    /// Index of the first [`Content`] in the store's content `Vec`.
    pub(crate) start: u32,
    /// Number of [`Content`] entries in the run.
    pub(crate) len: u32,
}

/// Opaque run of [`Segment`] owned by a [`crate::Snapshot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SegRange {
    /// Index of the first [`Segment`] in the store's segment `Vec`.
    pub(crate) start: u32,
    /// Number of [`Segment`] entries in the run.
    pub(crate) len: u32,
}

/// Owned backing store: the string interner plus the flat content / segment
/// `Vec`s the owned nodes' [`StrId`] / range payloads resolve against.
///
/// Not `Copy` (owns heap storage); not `PartialEq` (the interner's
/// `InternStats` field is not `PartialEq`).
#[derive(Debug, Clone, Default)]
pub(crate) struct NodeStore {
    base: Option<Arc<Self>>,
    string_offset: u32,
    content_offset: u32,
    segment_offset: u32,
    /// String interner backing every [`StrId`] in the tree.
    pub(crate) interner: StrInterner,
    /// Flat pool of [`Content`] entries; [`ContentRange`]s index here.
    contents: Vec<Content>,
    /// Flat pool of [`Segment`] entries; [`SegRange`]s index here.
    segments: Vec<Segment>,
}

impl NodeStore {
    pub(crate) fn layered(base: Arc<Self>) -> Self {
        let string_offset = base
            .string_offset
            .checked_add(u32::try_from(base.interner.len()).expect("string count fits u32"))
            .expect("string count fits u32");
        let content_offset = base
            .content_offset
            .checked_add(u32::try_from(base.contents.len()).expect("content count fits u32"))
            .expect("content count fits u32");
        let segment_offset = base
            .segment_offset
            .checked_add(u32::try_from(base.segments.len()).expect("segment count fits u32"))
            .expect("segment count fits u32");
        Self {
            base: Some(base),
            string_offset,
            content_offset,
            segment_offset,
            interner: StrInterner::default(),
            contents: Vec::new(),
            segments: Vec::new(),
        }
    }

    /// Empty store.
    #[cfg(test)]
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Intern `s` into the store's interner, returning a stable [`StrId`].
    pub(crate) fn intern(&mut self, s: &str) -> StrId {
        let local = self.interner.intern(s);
        StrId(
            self.string_offset
                .checked_add(local.0)
                .expect("string count fits u32"),
        )
    }

    /// Resolve a [`StrId`] to its interned bytes.
    ///
    /// # Panics
    ///
    /// Panics if `id` was not produced by this store's interner.
    #[must_use]
    pub(crate) fn resolve_str(&self, id: StrId) -> &str {
        let mut store = self;
        loop {
            if id.0 >= store.string_offset {
                return store.interner.resolve(StrId(id.0 - store.string_offset));
            }
            store = store.base.as_deref().expect("string id belongs to store");
        }
    }

    /// Append a content run and return the [`ContentRange`] that addresses it.
    ///
    /// # Panics
    ///
    /// Panics if the content pool would exceed `u32::MAX` entries — not
    /// reachable for any realistic document.
    pub(crate) fn push_contents(&mut self, items: &[Content]) -> ContentRange {
        let start = self
            .content_offset
            .checked_add(
                u32::try_from(self.contents.len()).expect("content pool exceeds u32 entry count"),
            )
            .expect("content pool exceeds u32 entry count");
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
    pub(crate) fn push_segments(&mut self, items: &[Segment]) -> SegRange {
        let start = self
            .segment_offset
            .checked_add(
                u32::try_from(self.segments.len()).expect("segment pool exceeds u32 entry count"),
            )
            .expect("segment pool exceeds u32 entry count");
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
    pub(crate) fn resolve_content_range(&self, range: ContentRange) -> &[Content] {
        let mut store = self;
        loop {
            if range.start >= store.content_offset {
                let start = (range.start - store.content_offset) as usize;
                return &store.contents[start..start + range.len as usize];
            }
            store = store
                .base
                .as_deref()
                .expect("content range belongs to store");
        }
    }

    /// Resolve a [`SegRange`] to its sub-slice of the segment pool.
    ///
    /// # Panics
    ///
    /// Panics if the range falls outside the segment pool.
    #[must_use]
    pub(crate) fn resolve_seg_range(&self, range: SegRange) -> &[Segment] {
        let mut store = self;
        loop {
            if range.start >= store.segment_offset {
                let start = (range.start - store.segment_offset) as usize;
                return &store.segments[start..start + range.len as usize];
            }
            store = store
                .base
                .as_deref()
                .expect("segment range belongs to store");
        }
    }

    /// Plain-text fast path over a length-1 content run: `Some(text)` iff the
    /// run is exactly one [`Content::Plain`]; `None` for a `Segments` run
    /// or any `len != 1`.
    ///
    /// Consumers reading a single-`Content` field (ruby base/reading,
    /// forward-format target, …) take its plain text through this.
    ///
    /// # Panics
    ///
    /// Panics if `range` falls outside the content pool (via
    /// [`Self::resolve_content_range`]).
    #[must_use]
    pub(crate) fn content_range_as_plain(&self, range: ContentRange) -> Option<&str> {
        match self.resolve_content_range(range) {
            [Content::Plain(id)] => Some(self.resolve_str(*id)),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn inherits_from(&self, expected: &Arc<Self>) -> bool {
        let mut base = self.base.as_ref();
        while let Some(store) = base {
            if Arc::ptr_eq(store, expected) {
                return true;
            }
            base = store.base.as_ref();
        }
        false
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
        let first = store.push_contents(&[Content::Plain(a)]);
        let second = store.push_contents(&[Content::Plain(b), Content::Plain(c)]);

        assert_eq!(first, ContentRange { start: 0, len: 1 });
        assert_eq!(second, ContentRange { start: 1, len: 2 });
        assert_eq!(store.resolve_content_range(first), &[Content::Plain(a)]);
        assert_eq!(
            store.resolve_content_range(second),
            &[Content::Plain(b), Content::Plain(c)]
        );

        // Segment ranges resolve independently.
        let seg = store.push_segments(&[Segment::Text(a), Segment::Text(c)]);
        assert_eq!(seg, SegRange { start: 0, len: 2 });
        assert_eq!(
            store.resolve_seg_range(seg),
            &[Segment::Text(a), Segment::Text(c)]
        );
    }

    #[test]
    fn segment_ranges_resolve_in_a_layered_store() {
        let mut base = NodeStore::new();
        let a = base.intern("a");
        let b = base.intern("b");
        base.push_segments(&[Segment::Text(a), Segment::Text(b)]);

        let mut layered = NodeStore::layered(Arc::new(base));
        let c = layered.intern("c");
        let d = layered.intern("d");
        let range = layered.push_segments(&[Segment::Text(c), Segment::Text(d)]);

        assert_eq!(range, SegRange { start: 2, len: 2 });
        assert_eq!(
            layered.resolve_seg_range(range),
            &[Segment::Text(c), Segment::Text(d)],
        );
    }

    #[test]
    fn content_range_as_plain_length_one() {
        let mut store = NodeStore::new();
        let a = store.intern("foo");
        let b = store.intern("bar");

        // Length-1 `Plain` run → the interned text (the 99%+ majority case).
        let plain = store.push_contents(&[Content::Plain(a)]);
        assert_eq!(store.content_range_as_plain(plain), Some("foo"));

        // A `Segments` run → `None` (mixed content; not the length-1 plain fast path).
        let seg = store.push_segments(&[Segment::Text(a)]);
        let mixed = store.push_contents(&[Content::Segments(seg)]);
        assert_eq!(store.content_range_as_plain(mixed), None);

        // A multi-entry run → `None` (only the length-1 fast path is plain).
        let two = store.push_contents(&[Content::Plain(a), Content::Plain(b)]);
        assert_eq!(store.content_range_as_plain(two), None);
    }
}
