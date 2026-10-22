//! Per-question analyses over a loaded [`crate::Trace`].

mod compare;
mod flame;
mod hot;
mod libs;
mod rollup;
mod stacks;

pub use compare::{ComparisonReport, ComparisonRow, compare};
pub use flame::{FoldedStack, folded_stacks, render_folded};
pub use hot::{HotMode, HotReport, HotRow, RowKind, hot_inclusive, hot_leaves};
pub use libs::{LibraryReport, LibraryRow, library_distribution};
pub use rollup::{RollupReport, RollupRow, rollup};
pub use stacks::{MatchedStack, MatchedStacksReport, matching_stacks};
