//! Paired-container classifier tags.
//!
//! [`ContainerKind`] is the tag the lexer's classify phase emits on
//! every paired open / close marker (e.g. `［＃ここから2字下げ］ … ［＃ここで字下げ終わり］`).
//! The renderer reads it when wrapping the enclosed sibling nodes
//! into an `AozoraNode::Container`.

/// The kinds of Aozora container blocks the lexer classifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ContainerKind {
    /// `［＃ここから N字下げ］`
    Indent { amount: u8 },
    /// `［＃割り注］ ... ［＃割り注終わり］` (when spanning multiple lines)
    Warichu,
    /// `［＃罫囲み］ ... ［＃罫囲み終わり］`
    Keigakomi,
    /// `［＃ここから地付き］` / `［＃ここから地から N 字上げ］`
    AlignEnd { offset: u8 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_kind_is_copy_and_fits_in_a_word() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<ContainerKind>();
        // u8 + discriminant, must fit in a few bytes so downstream
        // vector entries stay tight.
        assert!(size_of::<ContainerKind>() <= 4);
    }
}
