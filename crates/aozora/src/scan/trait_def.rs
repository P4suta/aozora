use std::vec::Vec;

use bumpalo::collections::Vec as BumpVec;

pub(crate) trait OffsetSink {
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
