#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Freshness {
    Fresh { elapsed: u64 },
    Stale { elapsed: u64 },
    Future,
}

pub(crate) const SECONDS_PER_DAY: u64 = 86_400;
pub(crate) const FRESH_SECONDS: u64 = 7 * SECONDS_PER_DAY;

pub(crate) const fn freshness(now: u64, checked: u64) -> Freshness {
    let Some(age) = now.checked_sub(checked) else {
        return Freshness::Future;
    };
    if age <= FRESH_SECONDS {
        Freshness::Fresh { elapsed: age }
    } else {
        Freshness::Stale { elapsed: age }
    }
}

#[cfg(kani)]
mod proofs {
    use super::{FRESH_SECONDS, Freshness, freshness};

    #[kani::proof]
    fn timestamps_require_a_nonfuture_check_within_the_exact_freshness_window() {
        let now: u64 = kani::any();
        let checked: u64 = kani::any();
        let result = freshness(now, checked);
        assert_eq!(result == Freshness::Future, checked > now);
        if checked <= now {
            let elapsed = now - checked;
            assert_eq!(
                matches!(result, Freshness::Fresh { .. }),
                elapsed <= FRESH_SECONDS
            );
            let age = match result {
                Freshness::Fresh { elapsed } | Freshness::Stale { elapsed } => elapsed,
                Freshness::Future => unreachable!(),
            };
            assert_eq!(age, elapsed);
        }
        kani::cover!(matches!(result, Freshness::Fresh { .. }));
        kani::cover!(matches!(result, Freshness::Stale { .. }));
        kani::cover!(result == Freshness::Future);
    }
}

#[cfg(kani)]
mod rejecting_controls {
    use super::{FRESH_SECONDS, Freshness, freshness};

    #[kani::proof]
    fn reject_a_future_timestamp_as_fresh() {
        assert!(matches!(freshness(0, 1), Freshness::Fresh { .. }));
    }

    #[kani::proof]
    fn reject_an_extra_second_beyond_the_freshness_window() {
        assert!(matches!(
            freshness(FRESH_SECONDS + 1, 0),
            Freshness::Fresh { .. }
        ));
    }
}
