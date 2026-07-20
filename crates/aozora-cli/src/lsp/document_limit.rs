pub(crate) const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;

#[inline]
pub(crate) const fn exceeds_document_cap(len: usize) -> bool {
    len > MAX_DOCUMENT_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_is_inclusive() {
        assert!(!exceeds_document_cap(MAX_DOCUMENT_BYTES));
        assert!(exceeds_document_cap(MAX_DOCUMENT_BYTES + 1));
    }
}
