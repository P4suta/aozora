//! Sorted-key map with cache-friendly binary search.
//!
//! Pairs an [`EytzingerArray`] of keys with a parallel `Vec` of values
//! laid out in matching Eytzinger order, so a single search returns
//! both the key index and the associated value with no second
//! indirection.

use alloc::vec::Vec;

use crate::eytzinger::EytzingerArray;

/// Sorted-key, value-bearing map. Lookup by key is O(log n) with the
/// cache-friendliness of [`EytzingerArray`].
///
/// Construction takes a sorted-ascending iterator of `(key, value)`
/// pairs; both keys and values are reordered into Eytzinger layout in
/// lockstep, so the matching `value` lives at the same index as its
/// `key`.
#[derive(Debug, Clone)]
pub struct EytzingerMap<K, V> {
    keys: EytzingerArray<K>,
    /// Values in Eytzinger order: `values[k]` corresponds to the key
    /// at Eytzinger index `k`. Constructed lock-step with the keys.
    values: Vec<V>,
}

impl<K, V> EytzingerMap<K, V> {
    /// Empty map.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            keys: EytzingerArray::new(),
            values: Vec::new(),
        }
    }

    /// Number of (key, value) pairs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// True iff zero pairs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

impl<K, V> Default for EytzingerMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Ord, V> EytzingerMap<K, V> {
    /// Look up `key`. Returns the matching value reference, or `None`.
    #[must_use]
    pub fn get(&self, key: &K) -> Option<&V> {
        let eytz = self.keys.search(key)?;
        // The keys array is the source of truth for length and bounds;
        // values is built in lockstep, so this index is always valid.
        self.values.get(eytz)
    }

    /// True iff `key` is present.
    #[must_use]
    pub fn contains_key(&self, key: &K) -> bool {
        self.keys.contains(key)
    }
}

impl<K: Ord + Clone, V: Clone> EytzingerMap<K, V> {
    /// Construct from a sorted-ascending slice of `(key, value)` pairs.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if the keys are not sorted ascending.
    #[must_use]
    pub fn from_sorted_slice(sorted: &[(K, V)]) -> Self {
        debug_assert!(
            sorted.windows(2).all(|w| w[0].0 <= w[1].0),
            "input keys must be sorted ascending"
        );
        let n = sorted.len();
        if n == 0 {
            return Self::new();
        }

        // Build keys via the Eytzinger array's existing constructor —
        // and in parallel place values at the same Eytzinger index by
        // running the same in-order tree walk.
        let keys_only: Vec<K> = sorted.iter().map(|(k, _)| k.clone()).collect();
        let keys = EytzingerArray::from_sorted_slice(&keys_only);

        let mut values: Vec<Option<V>> = (0..n).map(|_| None).collect();
        let mut sorted_idx = 0usize;
        place_values(&mut values, sorted, &mut sorted_idx, 0);
        debug_assert_eq!(
            sorted_idx, n,
            "value placement must visit every slot exactly once"
        );

        Self {
            keys,
            values: values
                .into_iter()
                .map(|slot| slot.expect("every value slot was filled"))
                .collect(),
        }
    }

    /// Construct from a sorted-ascending vector of `(key, value)`
    /// pairs. Convenience wrapper over
    /// [`EytzingerMap::from_sorted_slice`] that consumes its input,
    /// leaving room for a future reuse-the-allocation optimisation.
    /// Today the impl just delegates to the slice form.
    #[must_use]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "API takes ownership to permit future allocation reuse"
    )]
    pub fn from_sorted_vec(sorted: Vec<(K, V)>) -> Self {
        Self::from_sorted_slice(&sorted)
    }

    /// Iterate `(key, value)` pairs in ascending key order. O(n).
    #[must_use]
    pub fn iter_sorted(&self) -> SortedIter<'_, K, V> {
        SortedIter::new(self)
    }
}

/// Mirror of `eytzinger_build` in the array crate, but for the value
/// side: places `sorted[*idx].1` at each Eytzinger node in lockstep.
fn place_values<K, V: Clone>(
    values: &mut [Option<V>],
    sorted: &[(K, V)],
    sorted_idx: &mut usize,
    node: usize,
) {
    let n = values.len();
    if node >= n {
        return;
    }
    place_values(values, sorted, sorted_idx, 2 * node + 1);
    values[node] = Some(sorted[*sorted_idx].1.clone());
    *sorted_idx += 1;
    place_values(values, sorted, sorted_idx, 2 * node + 2);
}

/// In-order iterator over an [`EytzingerMap`]. Yields `(key, value)`
/// pairs in ascending key order.
#[derive(Debug)]
pub struct SortedIter<'a, K, V> {
    map: &'a EytzingerMap<K, V>,
    stack: Vec<usize>,
    descend: Option<usize>,
}

impl<'a, K, V> SortedIter<'a, K, V> {
    fn new(map: &'a EytzingerMap<K, V>) -> Self {
        let descend = if map.is_empty() { None } else { Some(0) };
        Self {
            map,
            stack: Vec::new(),
            descend,
        }
    }
}

impl<'a, K, V> Iterator for SortedIter<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<(&'a K, &'a V)> {
        // See `EytzingerArray::SortedIter::next` for why no outer loop
        // is needed — same descend-and-pop cycle, paired key + value.
        let n = self.map.len();
        while let Some(node) = self.descend
            && node < n
        {
            self.stack.push(node);
            self.descend = Some(2 * node + 1);
        }
        self.descend = None;

        let node = self.stack.pop()?;
        self.descend = Some(2 * node + 2);
        // Both arrays are length n by construction.
        let k = self.map.keys.get(node)?;
        let v = self.map.values.get(node)?;
        Some((k, v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn empty_map_lookups_miss() {
        let m: EytzingerMap<u32, &str> = EytzingerMap::new();
        assert!(m.is_empty());
        assert_eq!(m.len(), 0);
        assert_eq!(m.get(&5), None);
        assert!(!m.contains_key(&5));
    }

    #[test]
    fn three_element_lookup_returns_paired_value() {
        let m = EytzingerMap::from_sorted_slice(&[
            (10u32, "ten"),
            (20, "twenty"),
            (30, "thirty"),
        ]);
        assert_eq!(m.get(&10), Some(&"ten"));
        assert_eq!(m.get(&20), Some(&"twenty"));
        assert_eq!(m.get(&30), Some(&"thirty"));
        assert_eq!(m.get(&15), None);
    }

    #[test]
    fn iter_sorted_yields_input_order() {
        let pairs: Vec<(u32, u32)> =
            vec![(1, 100), (2, 200), (3, 300), (4, 400), (5, 500), (6, 600), (7, 700)];
        let m = EytzingerMap::from_sorted_slice(&pairs);
        let walked: Vec<(u32, u32)> = m.iter_sorted().map(|(k, v)| (*k, *v)).collect();
        assert_eq!(walked, pairs);
    }

    #[test]
    fn lookup_holds_for_each_pair_at_various_sizes() {
        for n in 0u32..32 {
            let pairs: Vec<(u32, u32)> = (0..n).map(|i| (i * 2, i * 100)).collect();
            let m = EytzingerMap::from_sorted_slice(&pairs);
            for (k, v) in &pairs {
                assert_eq!(m.get(k), Some(v), "size={n} key={k}");
            }
            // Odd numbers (gaps in the key set) must miss.
            for k in 0..n {
                assert_eq!(m.get(&(k * 2 + 1)), None, "size={n} odd key={}", k * 2 + 1);
            }
        }
    }

    #[test]
    fn from_sorted_vec_consumes_input() {
        let m = EytzingerMap::from_sorted_vec(vec![(1u32, 'a'), (2, 'b'), (3, 'c')]);
        assert_eq!(m.len(), 3);
        assert_eq!(m.get(&2), Some(&'b'));
    }

    #[test]
    #[should_panic(expected = "sorted ascending")]
    fn debug_panic_on_unsorted_keys() {
        // Body is the panic, not the return value — drop explicitly.
        drop(EytzingerMap::from_sorted_slice(&[(3u32, ()), (1, ())]));
    }

    #[test]
    fn default_is_empty() {
        let m: EytzingerMap<u32, ()> = EytzingerMap::default();
        assert!(m.is_empty());
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::collection::vec as vec_strategy;
    use proptest::prelude::*;

    proptest! {
        /// Map lookup must agree with HashMap-equivalent ground truth.
        #[test]
        fn map_get_matches_linear_lookup(
            mut pairs in vec_strategy((0u32..1_000, any::<u32>()), 0..200),
            probes in vec_strategy(0u32..1_000, 0..50),
        ) {
            // Deduplicate by key (last write wins) and sort.
            pairs.sort_by_key(|(k, _)| *k);
            pairs.dedup_by_key(|(k, _)| *k);
            let m = EytzingerMap::from_sorted_slice(&pairs);
            for probe in probes {
                let got = m.get(&probe).copied();
                let expected = pairs.iter().find(|(k, _)| *k == probe).map(|(_, v)| *v);
                prop_assert_eq!(got, expected);
            }
        }

        /// Sorted iteration must yield the original input order.
        #[test]
        fn iter_sorted_round_trips(
            mut pairs in vec_strategy((0u32..1_000, any::<u32>()), 0..200),
        ) {
            pairs.sort_by_key(|(k, _)| *k);
            pairs.dedup_by_key(|(k, _)| *k);
            let m = EytzingerMap::from_sorted_slice(&pairs);
            let walked: Vec<(u32, u32)> = m.iter_sorted().map(|(k, v)| (*k, *v)).collect();
            prop_assert_eq!(walked, pairs);
        }
    }
}
