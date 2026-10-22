//! Reports hot functions through [`hot_leaves`] and [`hot_inclusive`].

use std::collections::{HashMap, HashSet};

use crate::render::{Align, Column, TableBuilder};
use crate::{TableRenderable, Trace};

const TRAMPOLINE_MIN_INCLUSIVE_PCT: f64 = 95.0;
const LEAF_HOT_MIN_RATIO: f64 = 0.9;
const HOT_MIN_RATIO: f64 = 0.5;
const PERCENT_SCALE: f64 = 100.0;
const RANK_COLUMN_WIDTH: usize = 4;
const KIND_COLUMN_WIDTH: usize = 4;
const PERCENT_COLUMN_WIDTH: usize = 7;
const SAMPLES_COLUMN_WIDTH: usize = 8;

/// Aggregated top-N hot-frame report.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotReport {
    /// Whether counts include only leaves or all frames.
    pub mode: HotMode,
    /// Total weight of samples across the trace.
    pub total_samples: u64,
    /// Functions sorted by descending inclusive weight of samples.
    pub rows: Vec<HotRow>,
}

/// Which counting view a [`HotReport`] used.
#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HotMode {
    /// Count only leaf frames.
    Leaf,
    /// Count every distinct frame on sampled stacks.
    Inclusive,
}

/// One function's entry in a [`HotReport`].
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotRow {
    /// Resolved function name or hexadecimal address.
    pub label: String,
    /// Classification by the shares of samples and the label's resolution status.
    pub kind: RowKind,
    /// Weight of samples with this function anywhere on the stack.
    pub incl_samples: u64,
    /// `incl_samples` as a percentage of [`HotReport::total_samples`].
    pub incl_pct: f64,
    /// Weight of samples with this function as the leaf frame.
    /// Equals `incl_samples` for [`HotMode::Leaf`].
    pub self_samples: u64,
    /// `self_samples` as a percentage of [`HotReport::total_samples`].
    pub self_pct: f64,
}

/// Classification of a function's contribution to the sampled work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RowKind {
    /// Leaf samples dominate the function's inclusive total.
    LeafHot,
    /// At least half of inclusive samples are leaf samples.
    Hot,
    /// Most samples pass through the function to another frame.
    Wrapper,
    /// An entry function with high inclusive weight and low leaf weight.
    Trampoline,
    /// An address without a resolved function name.
    Unresolved,
}

impl RowKind {
    /// Returns the ASCII tag used in rendered tables.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::LeafHot => "LH",
            Self::Hot => "HW",
            Self::Wrapper => "WR",
            Self::Trampoline => "EP",
            Self::Unresolved => "??",
        }
    }
}

fn classify_row_kind(label: &str, self_pct: f64, incl_pct: f64) -> RowKind {
    if label.starts_with("0x") {
        return RowKind::Unresolved;
    }
    if incl_pct >= TRAMPOLINE_MIN_INCLUSIVE_PCT && self_pct < 1.0 {
        return RowKind::Trampoline;
    }
    if incl_pct == 0.0 {
        return RowKind::LeafHot;
    }
    let leaf_ratio = self_pct / incl_pct;
    if leaf_ratio >= LEAF_HOT_MIN_RATIO {
        RowKind::LeafHot
    } else if leaf_ratio >= HOT_MIN_RATIO {
        RowKind::Hot
    } else {
        RowKind::Wrapper
    }
}

/// Returns at most `n` hot leaf functions, sorted by descending weight of samples.
/// Inclusive and self counts are equal in this view.
#[must_use]
pub fn hot_leaves(trace: &Trace, n: usize) -> HotReport {
    let (leaf_counts, total) = collect_leaf_counts(trace);
    let mut rows: Vec<HotRow> = leaf_counts
        .into_iter()
        .map(|(label, samples)| {
            let pct = pct_of(samples, total);
            HotRow {
                kind: classify_row_kind(&label, pct, pct),
                label,
                incl_samples: samples,
                incl_pct: pct,
                self_samples: samples,
                self_pct: pct,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        b.incl_samples
            .cmp(&a.incl_samples)
            .then_with(|| a.label.cmp(&b.label))
    });
    rows.truncate(n);
    HotReport {
        mode: HotMode::Leaf,
        total_samples: total,
        rows,
    }
}

/// Returns at most `n` functions, sorted by descending inclusive weight of samples.
/// Each frame index counts once per sampled stack.
/// Self counts include only leaf samples.
#[must_use]
pub fn hot_inclusive(trace: &Trace, n: usize) -> HotReport {
    let (leaf_counts, total) = collect_leaf_counts(trace);
    let incl_counts = collect_inclusive_counts(trace);

    let mut rows: Vec<HotRow> = incl_counts
        .into_iter()
        .map(|(label, incl_samples)| {
            let self_samples = leaf_counts.get(&label).copied().unwrap_or(0);
            let incl_pct = pct_of(incl_samples, total);
            let self_pct = pct_of(self_samples, total);
            HotRow {
                kind: classify_row_kind(&label, self_pct, incl_pct),
                label,
                incl_samples,
                incl_pct,
                self_samples,
                self_pct,
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        b.incl_samples
            .cmp(&a.incl_samples)
            .then_with(|| a.label.cmp(&b.label))
    });
    rows.truncate(n);
    HotReport {
        mode: HotMode::Inclusive,
        total_samples: total,
        rows,
    }
}

fn collect_leaf_counts(trace: &Trace) -> (HashMap<String, u64>, u64) {
    let mut counts: HashMap<String, u64> = HashMap::new();
    let mut total = 0u64;
    for thread in &trace.threads {
        for sample in &thread.samples {
            total += sample.weight;
            let Some(stack_idx) = sample.stack_idx else {
                continue;
            };
            let Some(entry) = thread.stack_table.get(stack_idx) else {
                continue;
            };
            let label = thread.frame_label(entry.frame_idx);
            *counts.entry(label).or_insert(0) += sample.weight;
        }
    }
    (counts, total)
}

fn collect_inclusive_counts(trace: &Trace) -> HashMap<String, u64> {
    let mut counts: HashMap<String, u64> = HashMap::new();
    let mut seen: HashSet<usize> = HashSet::new();
    for thread in &trace.threads {
        for sample in &thread.samples {
            let Some(stack_idx) = sample.stack_idx else {
                continue;
            };
            seen.clear();
            for frame_idx in thread.walk_stack(stack_idx) {
                if seen.insert(frame_idx) {
                    let label = thread.frame_label(frame_idx);
                    *counts.entry(label).or_insert(0) += sample.weight;
                }
            }
        }
    }
    counts
}

#[expect(
    clippy::cast_precision_loss,
    reason = "sample shares are rendered as approximate percentages"
)]
fn pct_of(samples: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (samples as f64 / total as f64) * PERCENT_SCALE
    }
}

impl TableRenderable for HotReport {
    fn render_table(&self) -> String {
        let mode_label = match self.mode {
            HotMode::Leaf => "HOT LEAF",
            HotMode::Inclusive => "HOT INCLUSIVE",
        };
        let title = format!(
            "Top {} {} frames ({} total samples)",
            self.rows.len(),
            mode_label,
            self.total_samples,
        );
        let mut t = TableBuilder::new(
            title,
            vec![
                Column {
                    header: "rank".into(),
                    align: Align::Right,
                    width: RANK_COLUMN_WIDTH,
                },
                Column {
                    header: "kind".into(),
                    align: Align::Left,
                    width: KIND_COLUMN_WIDTH,
                },
                Column {
                    header: "incl %".into(),
                    align: Align::Right,
                    width: PERCENT_COLUMN_WIDTH,
                },
                Column {
                    header: "self %".into(),
                    align: Align::Right,
                    width: PERCENT_COLUMN_WIDTH,
                },
                Column {
                    header: "samples".into(),
                    align: Align::Right,
                    width: SAMPLES_COLUMN_WIDTH,
                },
                Column {
                    header: "function".into(),
                    align: Align::Left,
                    width: 0,
                },
            ],
        );
        for (i, r) in self.rows.iter().enumerate() {
            t.push_row(vec![
                format!("{}", i + 1),
                r.kind.tag().into(),
                format!("{:.2}", r.incl_pct),
                format!("{:.2}", r.self_pct),
                format!("{}", r.incl_samples),
                r.label.clone(),
            ]);
        }
        let mut out = t.render();
        out.push('\n');
        out.push_str(
            "  kind legend:  LH = leaf-hot (CPU was here)        HW = hot work (mostly leaf)\n\
             \x20              WR = wrapper / dispatcher (rarely leaf)  EP = entry-point trampoline (no own work)\n\
             \x20              ?? = unresolved address (libc internal or stripped binary frame)\n",
        );
        out
    }
}
