//! Cross-validate [`EytzingerMap::get`] against [`BTreeMap::get`].
//!
//! `EytzingerMap` is the lookup side of the borrowed registry — every
//! renderer hit on a sentinel codepoint resolves through one of these.
//! The decisive property here: for any sorted `[(K, V)]` input, the
//! Eytzinger map agrees with `BTreeMap` on `get` for every probe.
//!
//! Mutations the property kills:
//!
//! * `<` ↔ `<=` swaps in the search comparison (keys lost or
//!   duplicated near boundaries).
//! * Off-by-one in `2*k+1` / `2*k+2` (every probe near a subtree split
//!   silently returns the wrong value).
//! * `&V` returned from the wrong row (lookup phase reaches the right
//!   index but reads the neighbour's payload).

use std::collections::BTreeMap;

use aozora_proptest::config::default_config;
use aozora_veb::EytzingerMap;
use proptest::prelude::*;

fn build_sorted_unique_pairs(mut pairs: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    pairs.sort_by_key(|&(k, _)| k);
    pairs.dedup_by_key(|p| p.0);
    pairs
}

proptest! {
    #![proptest_config(default_config())]

    /// `EytzingerMap::get(k)` agrees with `BTreeMap::get(k)` for every
    /// member key and every adversarial probe. Disagreement means the
    /// map has lost an entry, returned the wrong neighbour's value,
    /// or fired off-by-one near a subtree boundary.
    #[test]
    fn get_matches_btreemap(
        pairs in prop::collection::vec((-1024_i32..=1024, any::<i32>()), 0..=64),
        probes in prop::collection::vec(-1100_i32..=1100, 1..=32),
    ) {
        let sorted = build_sorted_unique_pairs(pairs);
        let oracle: BTreeMap<i32, i32> = sorted.iter().copied().collect();
        let map = EytzingerMap::from_sorted_slice(&sorted);

        for &(key, _) in &sorted {
            prop_assert_eq!(
                map.get(&key).copied(),
                oracle.get(&key).copied(),
                "member key {} disagreement (sorted={:?})",
                key, sorted
            );
        }
        for &probe in &probes {
            prop_assert_eq!(
                map.get(&probe).copied(),
                oracle.get(&probe).copied(),
                "probe key {} disagreement (sorted={:?})",
                probe, sorted
            );
        }
    }

    /// `contains_key` is consistent with `get` returning `Some`. They
    /// share traversal code today but the relationship is invariant —
    /// any future split of the two paths must keep them aligned.
    #[test]
    fn contains_key_is_consistent_with_get(
        pairs in prop::collection::vec((-1024_i32..=1024, any::<i32>()), 0..=64),
        probe in -1100_i32..=1100,
    ) {
        let sorted = build_sorted_unique_pairs(pairs);
        let map = EytzingerMap::from_sorted_slice(&sorted);
        prop_assert_eq!(map.contains_key(&probe), map.get(&probe).is_some());
    }

    /// `iter_sorted` yields keys in ascending order — the renderer
    /// relies on this for sentinel sweeps that walk the registry.
    /// A regression here surfaces as out-of-order render output.
    #[test]
    fn iter_sorted_is_strictly_ascending(
        pairs in prop::collection::vec((-1024_i32..=1024, any::<i32>()), 0..=64),
    ) {
        let sorted = build_sorted_unique_pairs(pairs);
        let map = EytzingerMap::from_sorted_slice(&sorted);
        let keys: Vec<i32> = map.iter_sorted().map(|(k, _)| *k).collect();
        for window in keys.windows(2) {
            prop_assert!(window[0] < window[1], "iter_sorted out-of-order: {:?}", keys);
        }
    }
}
