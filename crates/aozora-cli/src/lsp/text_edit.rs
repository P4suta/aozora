use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ByteEdit {
    pub range: Range<usize>,
    pub new_text: String,
}

impl ByteEdit {
    #[must_use]
    pub(super) const fn new(range: Range<usize>, new_text: String) -> Self {
        Self { range, new_text }
    }
}
