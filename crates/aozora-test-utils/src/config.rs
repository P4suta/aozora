//! Shared proptest configuration.
//!
//! Every property test in the aozora workspace should use
//! [`default_config`] so `AOZORA_PROPTEST_CASES` tunes the whole sweep
//! consistently. The default of 128 cases is a deliberate compromise
//! between catching regressions quickly during `just test` and keeping
//! the CI loop under a few seconds per proptest binary.
//!
//! Override sites:
//!
//! * `AOZORA_PROPTEST_CASES=16` for tight local iteration.
//! * `AOZORA_PROPTEST_CASES=4096` for pre-release deep sweeps
//!   (`just prop-deep`).

use std::env;

use proptest::prelude::ProptestConfig;
use proptest::test_runner::FileFailurePersistence;

/// Default [`ProptestConfig`] used across every aozora property test.
///
/// * `cases` defaults to 128 and can be overridden via the
///   `AOZORA_PROPTEST_CASES` environment variable; values that fail to
///   parse fall through to the default rather than panicking, because a
///   CI misconfiguration must never silently replace strict testing
///   with a permissive default.
/// * `max_shrink_iters` is held at 10 000 — the proptest default — so
///   shrinking converges on minimal failure cases without blowing the
///   per-run time budget.
/// * `failure_persistence` writes regressions into each test's
///   `proptest-regressions/` directory so a failure replays instantly
///   on the next run. The existing repo convention is to commit these
///   files alongside the tests; `aozora-test-utils` does not deviate.
#[must_use]
pub fn default_config() -> ProptestConfig {
    ProptestConfig {
        cases: env::var("AOZORA_PROPTEST_CASES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(128),
        max_shrink_iters: 10_000,
        failure_persistence: Some(Box::new(FileFailurePersistence::WithSource(
            "proptest-regressions",
        ))),
        ..ProptestConfig::default()
    }
}
