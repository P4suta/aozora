pub(crate) use crate::spec::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL,
};

pub(crate) mod classify;
pub(crate) mod pair;
pub(crate) mod sanitize;
pub(crate) mod token;
mod tokenize;

pub(crate) use classify::{ClassifiedSpan, SpanKind, classify_range};
pub(crate) use pair::{PairEvent, pair};
pub(crate) use sanitize::sanitize;
pub(crate) use token::Token;
pub(crate) use tokenize::tokenize;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentinel_constants_are_in_pua_range() {
        for &c in &[
            INLINE_SENTINEL,
            BLOCK_LEAF_SENTINEL,
            BLOCK_OPEN_SENTINEL,
            BLOCK_CLOSE_SENTINEL,
        ] {
            let code = u32::from(c);
            assert!(
                (0xE000..=0xF8FF).contains(&code),
                "{c:?} ({code:#06X}) must lie in Unicode PUA"
            );
        }
    }

    #[test]
    fn sentinel_constants_are_distinct() {
        let sentinels = [
            INLINE_SENTINEL,
            BLOCK_LEAF_SENTINEL,
            BLOCK_OPEN_SENTINEL,
            BLOCK_CLOSE_SENTINEL,
        ];
        for (i, a) in sentinels.iter().enumerate() {
            for b in &sentinels[i + 1..] {
                assert_ne!(a, b, "sentinels must be pairwise distinct");
            }
        }
    }
}
