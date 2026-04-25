//! Phase 5 — binary-search lookup API over the Phase 4 registry.
//!
//! [`PlaceholderRegistry`] is built as four parallel `Vec<(u32, T)>`
//! by Phase 4, sorted by construction because the normalizer only
//! appends at the end of the output buffer. Phase 5 layers
//! `binary_search_by_key`–based accessors on top so
//! `afm-parser::post_process` and diagnostic consumers can resolve
//! a sentinel's position to its original classification in
//! `O(log N)` without cloning or re-sorting.
//!
//! The module holds *only* accessor impls and (cfg-test) coverage —
//! the storage type itself lives in [`crate::phase4_normalize`].

use aozora_syntax::{AozoraNode, ContainerKind};

use crate::phase4_normalize::PlaceholderRegistry;

impl PlaceholderRegistry {
    /// Resolve an inline sentinel position to its classified node.
    /// Returns `None` if `pos` is not the start of a recorded inline
    /// sentinel.
    #[must_use]
    pub fn inline_at(&self, pos: u32) -> Option<&AozoraNode> {
        let idx = self.inline.binary_search_by_key(&pos, |(p, _)| *p).ok()?;
        Some(&self.inline[idx].1)
    }

    /// Resolve a block-leaf sentinel position to its classified node.
    #[must_use]
    pub fn block_leaf_at(&self, pos: u32) -> Option<&AozoraNode> {
        let idx = self
            .block_leaf
            .binary_search_by_key(&pos, |(p, _)| *p)
            .ok()?;
        Some(&self.block_leaf[idx].1)
    }

    /// Resolve a block-open sentinel position to its container kind.
    #[must_use]
    pub fn block_open_at(&self, pos: u32) -> Option<ContainerKind> {
        let idx = self
            .block_open
            .binary_search_by_key(&pos, |(p, _)| *p)
            .ok()?;
        Some(self.block_open[idx].1)
    }

    /// Resolve a block-close sentinel position to its container kind.
    #[must_use]
    pub fn block_close_at(&self, pos: u32) -> Option<ContainerKind> {
        let idx = self
            .block_close
            .binary_search_by_key(&pos, |(p, _)| *p)
            .ok()?;
        Some(self.block_close[idx].1)
    }

    /// Total number of sentinels recorded across all four kinds.
    ///
    /// Useful for cheap sanity checks in `post_process` — a zero count
    /// means the normalized text has no Aozora constructs at all.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inline.len() + self.block_leaf.len() + self.block_open.len() + self.block_close.len()
    }

    /// True if the registry is empty across all four kinds.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inline.is_empty()
            && self.block_leaf.is_empty()
            && self.block_open.is_empty()
            && self.block_close.is_empty()
    }

    /// Assert the internal sort invariant: every `Vec` is
    /// strictly-increasing by position.
    ///
    /// Phase 4 maintains this naturally; the check is a cheap
    /// defensive probe that Phase 6 can opt into, and a useful
    /// tool in tests.
    #[must_use]
    pub fn is_sorted_strictly(&self) -> bool {
        fn is_sorted<T>(v: &[(u32, T)]) -> bool {
            v.windows(2).all(|w| w[0].0 < w[1].0)
        }
        is_sorted(&self.inline)
            && is_sorted(&self.block_leaf)
            && is_sorted(&self.block_open)
            && is_sorted(&self.block_close)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phase1_events::tokenize;
    use crate::phase2_pair::pair;
    use crate::phase3_classify::classify;
    use crate::phase4_normalize::normalize;

    fn build(src: &str) -> PlaceholderRegistry {
        let tokens = tokenize(src);
        let pair_out = pair(&tokens);
        let classify_out = classify(&pair_out, src);
        normalize(&classify_out, src).registry
    }

    #[test]
    fn empty_registry_is_empty_and_len_zero() {
        let reg = build("plain text");
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        assert!(reg.is_sorted_strictly());
    }

    #[test]
    fn inline_at_finds_recorded_position() {
        let reg = build("｜漢《かん》");
        assert_eq!(reg.inline.len(), 1);
        let (pos, _) = reg.inline[0];
        assert!(reg.inline_at(pos).is_some());
    }

    #[test]
    fn inline_at_off_position_returns_none() {
        // Preceding "ab" ensures the sentinel lands at pos ≥ 2, so
        // both sides of the sentinel are inside the normalized range.
        let reg = build("ab｜漢《かん》");
        let (pos, _) = reg.inline[0];
        assert!(pos >= 2, "test precondition");
        // A position 1 byte off must miss.
        assert!(reg.inline_at(pos + 1).is_none());
        assert!(reg.inline_at(pos - 1).is_none());
    }

    #[test]
    fn block_leaf_at_finds_page_break() {
        let reg = build("［＃改ページ］");
        let (pos, _) = reg.block_leaf[0];
        assert!(reg.block_leaf_at(pos).is_some());
    }

    #[test]
    fn block_open_close_at_find_container_kinds() {
        let reg = build("［＃ここから字下げ］本文［＃ここで字下げ終わり］");
        let (open_pos, _) = reg.block_open[0];
        let (close_pos, _) = reg.block_close[0];
        assert!(matches!(
            reg.block_open_at(open_pos),
            Some(ContainerKind::Indent { .. })
        ));
        assert!(matches!(
            reg.block_close_at(close_pos),
            Some(ContainerKind::Indent { .. })
        ));
    }

    #[test]
    fn registry_len_counts_all_kinds() {
        let reg =
            build("｜a《b》｜c《d》［＃改ページ］［＃ここから字下げ］X［＃ここで字下げ終わり］");
        assert_eq!(reg.inline.len(), 2);
        assert_eq!(reg.block_leaf.len(), 1);
        assert_eq!(reg.block_open.len(), 1);
        assert_eq!(reg.block_close.len(), 1);
        assert_eq!(reg.len(), 5);
    }

    #[test]
    fn registry_remains_sorted_over_many_inline_spans() {
        // Rapid-fire inline ruby — registry positions must remain
        // strictly increasing.
        use std::fmt::Write;
        let mut src = String::new();
        for i in 0..20 {
            write!(&mut src, "｜漢{i}《かん{i}》").expect("write to String");
        }
        let reg = build(&src);
        assert_eq!(reg.inline.len(), 20);
        assert!(reg.is_sorted_strictly());
    }

    #[test]
    fn lookup_between_two_adjacent_sentinels_returns_correct_node() {
        let reg = build("｜A《a》｜B《b》");
        assert_eq!(reg.inline.len(), 2);
        let (pos0, _) = reg.inline[0];
        let (pos1, _) = reg.inline[1];
        // Each lookup must return distinct nodes (and not cross-
        // pollinate through stale indices).
        let n0 = reg.inline_at(pos0).expect("first");
        let n1 = reg.inline_at(pos1).expect("second");
        assert_ne!(n0, n1, "expected distinct nodes");
    }

    #[test]
    fn empty_registry_lookups_return_none() {
        let reg = build("");
        assert!(reg.inline_at(0).is_none());
        assert!(reg.block_leaf_at(0).is_none());
        assert!(reg.block_open_at(0).is_none());
        assert!(reg.block_close_at(0).is_none());
    }
}
