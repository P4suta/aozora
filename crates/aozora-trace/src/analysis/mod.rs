//! Per-question analyses over a loaded [`crate::Trace`].
//!
//! Every analysis is a single function that takes a `&Trace` (plus
//! options) and returns a typed report. Reports implement
//! [`crate::TableRenderable`] for plain-text dumping and are
//! serializable so callers can pipe them into JSON.
//!
//! Convention: a function `foo(trace, …)` produces a `FooReport`
//! whose `render_table()` yields the human form. Splits along these
//! lines so the CLI is a thin shell, and so cross-trace pipelines
//! (e.g. [`compare`]) can re-use the typed reports.

mod compare;
mod flame;
mod hot;
mod libs;
mod rollup;
mod stacks;

pub use compare::{ComparisonReport, ComparisonRow, compare};
pub use flame::{FoldedStack, folded_stacks, render_folded};
pub use hot::{HotReport, HotRow, hot_inclusive, hot_leaves};
pub use libs::{LibraryReport, LibraryRow, library_distribution};
pub use rollup::{RollupReport, RollupRow, rollup};
pub use stacks::{MatchedStack, MatchedStacksReport, matching_stacks};
