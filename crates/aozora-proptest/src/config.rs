//! Supplies shared property-test settings through [`default_config`].

use std::env;

use proptest::prelude::ProptestConfig;
use proptest::test_runner::FileFailurePersistence;

const DEFAULT_CASES: u32 = 128;
const MAX_SHRINK_ITERATIONS: u32 = 10_000;

/// Returns test settings with regression persistence and an optional `AOZORA_PROPTEST_CASES` override.
/// Missing or invalid overrides use the default case count.
#[must_use]
pub fn default_config() -> ProptestConfig {
    ProptestConfig {
        cases: env::var("AOZORA_PROPTEST_CASES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_CASES),
        max_shrink_iters: MAX_SHRINK_ITERATIONS,
        failure_persistence: Some(Box::new(FileFailurePersistence::WithSource(
            "proptest-regressions",
        ))),
        ..ProptestConfig::default()
    }
}
