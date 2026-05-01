//! Non-emptiness invariant for [`Content<'src>`].
//!
//! AST variants like [`Ruby`](super::Ruby) and [`Bouten`](super::Bouten)
//! semantically require non-empty content payloads (an empty ruby base
//! is a parse bug, not a valid state). Pre-Phase-E6 the field types
//! were plain `Content` and the invariant lived in prose comments.
//!
//! [`NonEmpty<Content<'src>>`] makes the invariant a build-time fact:
//! the `NonEmpty::new` constructor returns `Option`, so empty content
//! cannot enter the AST without a deliberate `unwrap` / `expect`. The
//! allocator (`aozora_syntax::alloc`) does the `expect` exactly once
//! per node variant — Phase 3's classifier guarantees the input is
//! non-empty, and an empty payload at allocation time signals a
//! pipeline-internal bug rather than valid input.
//!
//! Read access is via [`Deref`](core::ops::Deref) so existing
//! consumers of [`Content`] inherent methods (`as_plain`, `iter`)
//! work unchanged on `NonEmpty<Content>`.

use core::ops::Deref;

use super::types::Content;

/// Non-emptiness wrapper for an AST payload.
///
/// Only constructable through [`Self::new`] (returns `Option`) or
/// [`Self::new_unchecked`] (caller-asserted, used only by the
/// allocator after Phase 3 classification has guaranteed the
/// non-emptiness). Auto-derefs to the inner payload for read access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NonEmpty<T>(T);

impl<T> NonEmpty<T> {
    /// Construct without checking. Caller must guarantee the value is
    /// non-empty by some other means (typically: this is a pipeline
    /// allocator that just classified the payload as non-empty).
    ///
    /// Marked `#[doc(hidden)]` to discourage casual use; the typed
    /// constructor [`Self::new`] is the supported path for outside
    /// callers.
    #[doc(hidden)]
    #[must_use]
    pub const fn new_unchecked(value: T) -> Self {
        Self(value)
    }

    /// Consume the wrapper and return the inner payload.
    #[must_use]
    pub const fn into_inner(self) -> T
    where
        T: Copy,
    {
        self.0
    }

    /// Borrow the inner payload directly. Equivalent to dereferencing
    /// through [`Deref`].
    #[must_use]
    pub const fn as_inner(&self) -> &T {
        &self.0
    }
}

impl<T> Deref for NonEmpty<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<'src> NonEmpty<Content<'src>> {
    /// Construct, returning `None` if the content is empty.
    ///
    /// `Plain("")` and `Segments(&[])` are both rejected; everything
    /// else (including a single segment that happens to carry empty
    /// text) is accepted.
    #[must_use]
    pub fn new(content: Content<'src>) -> Option<Self> {
        match content {
            Content::Plain(s) if !s.is_empty() => Some(Self(content)),
            Content::Segments(segs) if !segs.is_empty() => Some(Self(content)),
            _ => None,
        }
    }

    /// Underlying [`Content`]. Convenience accessor for callers that
    /// want a `Copy`-style move rather than a deref-borrow.
    #[must_use]
    pub const fn get(self) -> Content<'src> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::Segment;
    use super::*;

    #[test]
    fn new_rejects_plain_empty_string() {
        assert!(NonEmpty::new(Content::Plain("")).is_none());
    }

    #[test]
    fn new_rejects_empty_segments_slice() {
        assert!(NonEmpty::new(Content::Segments(&[])).is_none());
    }

    #[test]
    fn new_accepts_plain_non_empty() {
        let ne = NonEmpty::new(Content::Plain("text")).expect("non-empty");
        assert!(matches!(ne.get(), Content::Plain("text")));
    }

    #[test]
    fn new_accepts_non_empty_segments() {
        static SEGS: &[Segment<'static>] = &[Segment::Text("a")];
        let ne = NonEmpty::new(Content::Segments(SEGS)).expect("non-empty");
        assert!(matches!(ne.get(), Content::Segments(s) if s.len() == 1));
    }

    #[test]
    fn deref_gives_inner_methods() {
        let ne = NonEmpty::new(Content::Plain("abc")).expect("non-empty");
        // Deref to Content lets us call inherent methods directly.
        assert_eq!(ne.as_plain(), Some("abc"));
    }

    #[test]
    fn empty_content_const_is_rejected() {
        assert!(NonEmpty::new(Content::EMPTY).is_none());
    }
}
