//! Cross-validate [`EytzingerArray::search`] against [`BTreeSet::contains`].
//!
//! The Eytzinger layout's lookup walks `k = 2 * k + 1` (left) /
//! `k = 2 * k + 2` (right) over the implicit binary-heap arrangement.
//! Off-by-one or branch swap on those indices is the most expressive
//! mutation a mutator could land — and the property test here is the
//! decisive killer: any swap of `+1` ↔ `+2`, `<=` ↔ `<`, or `Ordering`
//! arm reordering produces lookups that disagree with `BTreeSet`.
//!
//! Two angles:
//!
//! 1. Membership: every value in the source set is found; every value
//!    not in the set is reported missing.
//! 2. Stability: searching the same value twice on the same array
//!    yields the same byte-identical result — nondeterminism in the
//!    walk would surface as a flaky shrink.

use std::collections::BTreeSet;

use aozora_proptest::config::default_config;
use aozora_veb::EytzingerArray;
use proptest::prelude::*;

fn build_sorted_unique(mut xs: Vec<i64>) -> Vec<i64> {
    xs.sort_unstable();
    xs.dedup();
    xs
}

proptest! {
    #![proptest_config(default_config())]

    /// `EytzingerArray::search(x).is_some()` iff `BTreeSet::contains(&x)`,
    /// for every probe value (members + a swept range that catches
    /// near-miss off-by-ones). The Eytzinger layout's lookup must
    /// agree with `BTreeSet` on every input.
    #[test]
    fn search_membership_matches_btreeset(
        values in prop::collection::vec(-1024_i64..=1024, 0..=64),
        probes in prop::collection::vec(-1100_i64..=1100, 1..=32),
    ) {
        let sorted = build_sorted_unique(values);
        let oracle: BTreeSet<i64> = sorted.iter().copied().collect();
        let array = EytzingerArray::from_sorted_slice(&sorted);

        // Sweep over both the sorted set itself (positive controls)
        // and an arbitrary probe vector (negative + boundary controls).
        for &probe in sorted.iter().chain(probes.iter()) {
            let found = array.search(&probe).is_some();
            let expected = oracle.contains(&probe);
            prop_assert_eq!(
                found,
                expected,
                "membership disagreement for probe={} (sorted={:?})",
                probe, sorted
            );
        }
    }

    /// `search` returns the same result on repeated calls. A
    /// nondeterministic walk (e.g. interior mutation, RNG-seeded
    /// dispatch) would surface as a flaky shrink — by gating
    /// repeatability we promote it to a proptest fail.
    #[test]
    fn search_is_deterministic_under_repeats(
        values in prop::collection::vec(-1024_i64..=1024, 0..=64),
        probe in -1100_i64..=1100,
    ) {
        let sorted = build_sorted_unique(values);
        let array = EytzingerArray::from_sorted_slice(&sorted);
        let first = array.search(&probe);
        let second = array.search(&probe);
        prop_assert_eq!(first, second);
    }

    /// `from_sorted_slice` and `from_sorted_vec` build equivalent
    /// arrays. `iter_sorted` should yield the same sequence in both
    /// cases — divergence would mean the Vec-taking constructor has
    /// a different internal layout than the slice-taking one.
    #[test]
    fn slice_and_vec_constructors_agree(
        values in prop::collection::vec(-1024_i64..=1024, 0..=64),
    ) {
        let sorted = build_sorted_unique(values);
        let from_slice = EytzingerArray::from_sorted_slice(&sorted);
        let from_vec = EytzingerArray::from_sorted_vec(sorted);
        let a: Vec<i64> = from_slice.iter_sorted().copied().collect();
        let b: Vec<i64> = from_vec.iter_sorted().copied().collect();
        prop_assert_eq!(a, b);
    }
}
