use std::vec::Vec;

use crate::collections::eytzinger::EytzingerArray;

#[derive(Debug, Clone)]
pub(crate) struct EytzingerMap<K, V> {
    keys: EytzingerArray<K>,
    values: Vec<V>,
}

impl<K, V> EytzingerMap<K, V> {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self {
            keys: EytzingerArray::new(),
            values: Vec::new(),
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.keys.len()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

impl<K, V> Default for EytzingerMap<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Ord, V> EytzingerMap<K, V> {
    #[must_use]
    pub(crate) fn get(&self, key: &K) -> Option<&V> {
        let eytz = self.keys.search(key)?;
        self.values.get(eytz)
    }
}

impl<K: Ord + Clone, V: Clone> EytzingerMap<K, V> {
    #[must_use]
    pub(crate) fn from_sorted_slice(sorted: &[(K, V)]) -> Self {
        Self::from_sorted_by(sorted, Clone::clone)
    }

    pub(crate) fn from_sorted_by<T>(sorted: &[T], project: impl Fn(&T) -> (K, V) + Copy) -> Self {
        let n = sorted.len();
        if n == 0 {
            return Self::new();
        }

        let mut layout = vec![0; n];
        let sorted_idx = {
            let mut placer = IndexPlacer {
                layout: &mut layout,
                sorted_idx: 0,
            };
            placer.place(0);
            placer.sorted_idx
        };
        debug_assert_eq!(
            sorted_idx, n,
            "pair placement must visit every slot exactly once"
        );

        let mut keys = Vec::with_capacity(n);
        let mut values = Vec::with_capacity(n);
        for &index in &layout {
            let (key, value) = project(&sorted[index]);
            keys.push(key);
            values.push(value);
        }
        debug_assert!(
            keys_are_sorted_in_source_order(&keys, &layout),
            "input keys must be sorted ascending"
        );
        Self {
            keys: EytzingerArray::from_layout(keys),
            values,
        }
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn iter_sorted(&self) -> SortedIter<'_, K, V> {
        SortedIter::new(self)
    }
}

fn keys_are_sorted_in_source_order<K: Ord>(keys: &[K], layout: &[usize]) -> bool {
    let mut source_to_layout = vec![0; layout.len()];
    for (layout_index, &source_index) in layout.iter().enumerate() {
        source_to_layout[source_index] = layout_index;
    }
    source_to_layout
        .windows(2)
        .all(|window| keys[window[0]] <= keys[window[1]])
}

struct IndexPlacer<'a> {
    layout: &'a mut [usize],
    sorted_idx: usize,
}

impl IndexPlacer<'_> {
    fn place(&mut self, node: usize) {
        if node >= self.layout.len() {
            return;
        }
        self.place(2 * node + 1);
        self.layout[node] = self.sorted_idx;
        self.sorted_idx += 1;
        self.place(2 * node + 2);
    }
}

#[derive(Debug)]
#[cfg(test)]
pub(crate) struct SortedIter<'a, K, V> {
    map: &'a EytzingerMap<K, V>,
    stack: Vec<usize>,
    descend: Option<usize>,
}

#[cfg(test)]
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

#[cfg(test)]
impl<'a, K, V> Iterator for SortedIter<'a, K, V> {
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<(&'a K, &'a V)> {
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
        let k = self.map.keys.get(node)?;
        let v = self.map.values.get(node)?;
        Some((k, v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::vec;

    #[test]
    fn empty_map_lookups_miss() {
        let m: EytzingerMap<u32, &str> = EytzingerMap::new();
        assert!(m.is_empty());
        assert_eq!(m.len(), 0);
        assert_eq!(m.get(&5), None);
    }

    #[test]
    fn three_element_lookup_returns_paired_value() {
        let m = EytzingerMap::from_sorted_slice(&[(10u32, "ten"), (20, "twenty"), (30, "thirty")]);
        assert_eq!(m.get(&10), Some(&"ten"));
        assert_eq!(m.get(&20), Some(&"twenty"));
        assert_eq!(m.get(&30), Some(&"thirty"));
        assert_eq!(m.get(&15), None);
    }

    #[test]
    fn iter_sorted_yields_input_order() {
        let pairs: Vec<(u32, u32)> = vec![
            (1, 100),
            (2, 200),
            (3, 300),
            (4, 400),
            (5, 500),
            (6, 600),
            (7, 700),
        ];
        let m = EytzingerMap::from_sorted_slice(&pairs);
        let walked: Vec<(u32, u32)> = m.iter_sorted().map(|(k, v)| (*k, *v)).collect();
        assert_eq!(walked, pairs);
    }

    #[test]
    fn projection_runs_once_per_entry() {
        let calls = Cell::new(0);
        let pairs = [(10u32, "ten"), (20, "twenty"), (30, "thirty")];
        let map = EytzingerMap::from_sorted_by(&pairs, |pair| {
            calls.set(calls.get() + 1);
            *pair
        });

        assert_eq!(calls.get(), pairs.len());
        assert_eq!(map.get(&10), Some(&"ten"));
        assert_eq!(map.get(&20), Some(&"twenty"));
        assert_eq!(map.get(&30), Some(&"thirty"));
    }

    #[test]
    fn lookup_holds_for_each_pair_at_various_sizes() {
        for n in 0u32..32 {
            let pairs: Vec<(u32, u32)> = (0..n).map(|i| (i * 2, i * 100)).collect();
            let m = EytzingerMap::from_sorted_slice(&pairs);
            for (k, v) in &pairs {
                assert_eq!(m.get(k), Some(v), "size={n} key={k}");
            }
            for k in 0..n {
                assert_eq!(m.get(&(k * 2 + 1)), None, "size={n} odd key={}", k * 2 + 1);
            }
        }
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "sorted ascending")]
    fn debug_panic_on_unsorted_keys() {
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
        #[test]
        fn map_get_matches_linear_lookup(
            mut pairs in vec_strategy((0u32..1_000, any::<u32>()), 0..200),
            probes in vec_strategy(0u32..1_000, 0..50),
        ) {
            pairs.sort_by_key(|(k, _)| *k);
            pairs.dedup_by_key(|(k, _)| *k);
            let m = EytzingerMap::from_sorted_slice(&pairs);
            for probe in probes {
                let got = m.get(&probe).copied();
                let expected = pairs.iter().find(|(k, _)| *k == probe).map(|(_, v)| *v);
                prop_assert_eq!(got, expected);
            }
        }

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
