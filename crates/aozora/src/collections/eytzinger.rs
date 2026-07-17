//! Sorted array in Eytzinger (BFS-of-BST) layout.

use core::cmp::Ordering;
use std::vec::Vec;

/// Sorted collection optimized for cache-friendly binary search.
///
/// Construction is O(n); lookup is O(log n) with a memory access
/// pattern friendlier to the CPU prefetcher than `Vec::binary_search`.
/// See the crate-level documentation for the layout rationale.
///
/// # Generic over `T: Ord`
///
/// Any totally-ordered type works. The intended primary use is
/// `EytzingerArray<u32>` (byte-position lookup over a placeholder
/// registry), but the algorithm makes no assumption beyond `Ord`.
#[derive(Debug, Clone)]
pub(super) struct EytzingerArray<T> {
    /// Eytzinger-layout backing store. `data[0]` is the BST root,
    /// `data[1]` and `data[2]` are its children, and so on.
    data: Vec<T>,
}

impl<T> EytzingerArray<T> {
    /// Empty array.
    #[must_use]
    pub(super) const fn new() -> Self {
        Self { data: Vec::new() }
    }

    /// Number of elements.
    #[must_use]
    pub(super) fn len(&self) -> usize {
        self.data.len()
    }

    /// True iff zero elements.
    #[must_use]
    pub(super) fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Element at the given Eytzinger index, or `None` if out-of-range.
    ///
    /// The index is **the Eytzinger index** (i.e., the index returned
    /// by [`EytzingerArray::search`]), not a sorted-order index. Most
    /// callers want [`EytzingerMap::iter_sorted`](crate::collections::EytzingerMap::iter_sorted)
    /// instead for ordered traversal.
    #[must_use]
    pub(super) fn get(&self, eytz_index: usize) -> Option<&T> {
        self.data.get(eytz_index)
    }
}

impl<T> Default for EytzingerArray<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Ord> EytzingerArray<T> {
    /// Locate `target` in the array.
    ///
    /// Returns `Some(eytz_index)` if found, else `None`. The index is
    /// in **Eytzinger order**, not sorted order — pair it with
    /// [`EytzingerArray::get`] for value retrieval, or compose with an
    /// [`super::EytzingerMap`] which stores values in matching layout.
    #[must_use]
    pub(super) fn search(&self, target: &T) -> Option<usize> {
        let n = self.data.len();
        let mut k = 0usize;
        while k < n {
            let here = &self.data[k];
            match target.cmp(here) {
                Ordering::Less => k = 2 * k + 1,
                Ordering::Greater => k = 2 * k + 2,
                Ordering::Equal => return Some(k),
            }
        }
        None
    }
}

impl<T: Ord + Clone> EytzingerArray<T> {
    /// Construct from a sorted-ascending slice.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if `sorted` is not sorted ascending. In
    /// release builds the search invariant is silently violated for
    /// out-of-order input — verify the source upstream.
    #[must_use]
    pub(super) fn from_sorted_slice(sorted: &[T]) -> Self {
        debug_assert!(
            sorted.windows(2).all(|w| w[0] <= w[1]),
            "input must be sorted ascending"
        );
        let n = sorted.len();
        if n == 0 {
            return Self::new();
        }
        // The layout is a BFS of the implicit balanced BST. We build
        // it by an in-order traversal of the implicit tree, placing
        // sorted[i] at each node visited. This is O(n) work plus
        // O(log n) recursion depth.
        let mut data: Vec<Option<T>> = (0..n).map(|_| None).collect();
        let mut sorted_idx = 0usize;
        eytzinger_build(&mut data, sorted, &mut sorted_idx, 0);
        debug_assert_eq!(
            sorted_idx, n,
            "build must visit every Eytzinger slot exactly once"
        );
        Self {
            data: data
                .into_iter()
                .map(|slot| slot.expect("every Eytzinger slot was filled"))
                .collect(),
        }
    }
}

/// In-order traversal of the implicit Eytzinger tree, placing
/// `sorted[*sorted_idx]` at each node. The recursive call depth is
/// O(log n); the per-node work is O(1).
fn eytzinger_build<T: Clone>(
    data: &mut [Option<T>],
    sorted: &[T],
    sorted_idx: &mut usize,
    node: usize,
) {
    let n = data.len();
    if node >= n {
        return;
    }
    eytzinger_build(data, sorted, sorted_idx, 2 * node + 1);
    data[node] = Some(sorted[*sorted_idx].clone());
    *sorted_idx += 1;
    eytzinger_build(data, sorted, sorted_idx, 2 * node + 2);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_array_has_zero_len_and_no_searches_succeed() {
        let arr: EytzingerArray<u32> = EytzingerArray::new();
        assert_eq!(arr.len(), 0);
        assert!(arr.is_empty());
        assert_eq!(arr.search(&5), None);
    }

    #[test]
    fn single_element_array() {
        let arr = EytzingerArray::from_sorted_slice(&[42u32]);
        assert_eq!(arr.len(), 1);
        assert_eq!(arr.search(&42), Some(0));
        assert_eq!(arr.search(&41), None);
        assert_eq!(arr.search(&43), None);
    }

    #[test]
    fn seven_element_layout_matches_reference() {
        // [10,20,30,40,50,60,70] → [40,20,60,10,30,50,70]
        let arr = EytzingerArray::from_sorted_slice(&[10u32, 20, 30, 40, 50, 60, 70]);
        assert_eq!(arr.len(), 7);
        // Pin the actual layout via get() — root must be 40.
        assert_eq!(arr.get(0), Some(&40));
        assert_eq!(arr.get(1), Some(&20));
        assert_eq!(arr.get(2), Some(&60));
        assert_eq!(arr.get(3), Some(&10));
        assert_eq!(arr.get(4), Some(&30));
        assert_eq!(arr.get(5), Some(&50));
        assert_eq!(arr.get(6), Some(&70));
    }

    #[test]
    fn search_finds_every_element() {
        let inputs: &[u32] = &[5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55];
        let arr = EytzingerArray::from_sorted_slice(inputs);
        for &x in inputs {
            assert!(arr.search(&x).is_some(), "missing element {x}");
        }
    }

    #[test]
    fn search_misses_for_absent_targets() {
        let arr = EytzingerArray::from_sorted_slice(&[10u32, 20, 30, 40, 50]);
        assert_eq!(arr.search(&5), None);
        assert_eq!(arr.search(&15), None);
        assert_eq!(arr.search(&25), None);
        assert_eq!(arr.search(&35), None);
        assert_eq!(arr.search(&45), None);
        assert_eq!(arr.search(&55), None);
    }

    #[test]
    fn search_works_at_powers_of_two_boundaries() {
        // The Eytzinger layout's tree is "complete left-justified" —
        // shape changes at n = 2^k - 1 vs n = 2^k. Cover both.
        for n in [1u32, 2, 3, 4, 7, 8, 15, 16, 31, 32, 63, 64, 127, 128] {
            let inputs: Vec<u32> = (0..n).collect();
            let arr = EytzingerArray::from_sorted_slice(&inputs);
            for &x in &inputs {
                assert!(arr.search(&x).is_some(), "size {n}, missing {x}");
            }
            // Probe one above max — must miss.
            assert_eq!(arr.search(&(n + 1000)), None, "size {n}");
        }
    }

    #[test]
    #[should_panic(expected = "sorted ascending")]
    fn debug_panic_on_unsorted_input() {
        // Body is the panic, not the return value — drop explicitly.
        drop(EytzingerArray::from_sorted_slice(&[5u32, 3, 7]));
    }

    #[test]
    fn search_handles_duplicates_by_returning_some_match() {
        // Duplicates in the input are allowed (the sorted check uses
        // `<=`); search may return any one of them, but at least one.
        let arr = EytzingerArray::from_sorted_slice(&[1u32, 2, 2, 2, 3]);
        // Every duplicate target must be found at *some* index.
        let found = arr.search(&2).expect("at least one matching index");
        // The matching slot's value must equal the target.
        assert_eq!(arr.get(found), Some(&2));
    }

    #[test]
    fn default_is_empty() {
        let arr: EytzingerArray<u32> = EytzingerArray::default();
        assert!(arr.is_empty());
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::collection::vec as vec_strategy;
    use proptest::prelude::*;

    proptest! {
        /// Search must agree with linear scan for every input.
        #[test]
        fn search_matches_linear_scan(
            mut items in vec_strategy(0u32..1_000_000, 0..200),
            probes in vec_strategy(0u32..1_000_000, 0..50),
        ) {
            items.sort_unstable();
            items.dedup();
            let arr = EytzingerArray::from_sorted_slice(&items);
            for probe in probes {
                let eytz = arr.search(&probe);
                let linear = items.binary_search(&probe).ok();
                prop_assert_eq!(
                    eytz.and_then(|k| arr.get(k).copied()),
                    linear.map(|i| items[i]),
                );
            }
        }
    }
}
