//! Top-N hot frames — leaf or inclusive.
//!
//! Two views are provided over the same sample stream:
//!
//! - **Leaf** (`hot_leaves`): per-sample, count *only* the leaf-most
//!   frame. "Where the CPU is *literally* spending time right now."
//!   Best for tight-loop / victim-function analysis.
//! - **Inclusive** (`hot_inclusive`): per-sample, count *every*
//!   frame on the call stack (deduplicated per sample to avoid
//!   double-counting recursion). "Which functions are on the hot
//!   path." Best for entry-point analysis.
//!
//! ## Self vs Inclusive — disambiguating trampolines from hot work
//!
//! In the raw inclusive view the top of the table is dominated by
//! entry-point trampolines (`_start` → `__libc_start_main` →
//! `FnOnce::call_once` → `<binary>::main` → …) which all clock
//! ~99 % because every sample's stack passes through them. They
//! aren't doing any real work.
//!
//! Filtering them OUT loses information. Instead this report
//! shows BOTH columns:
//!
//! - `incl %` = inclusive percentage (frame appears anywhere on
//!   the stack)
//! - `self %` = leaf-frame percentage of the SAME function (where
//!   the CPU was when the sample fired)
//!
//! A trampoline shows up as `99 % incl, ~0 % self`; an actual hot
//! function shows up as `X % incl, X % self` (or close to it). The
//! user can scan `self %` to find real targets while still seeing
//! the call chain.

use std::collections::{HashMap, HashSet};

use crate::render::{Align, Column, TableBuilder};
use crate::{TableRenderable, Trace};

/// Aggregated top-N hot-frame report.
#[derive(Debug, Clone)]
pub struct HotReport {
    pub mode: HotMode,
    pub total_samples: u64,
    pub rows: Vec<HotRow>,
}

#[derive(Debug, Clone, Copy)]
pub enum HotMode {
    Leaf,
    Inclusive,
}

#[derive(Debug, Clone)]
pub struct HotRow {
    pub label: String,
    /// Display annotation: `[entry]`, `[trampoline]`, `[unresolved]`,
    /// or empty for a normal function. Computed by `classify_row_kind`.
    pub kind: RowKind,
    /// Inclusive: the frame appears anywhere on the stack.
    pub incl_samples: u64,
    pub incl_pct: f64,
    /// Self: the frame is the leaf of the stack (where the CPU was).
    /// For `HotMode::Leaf` this equals `incl_*`.
    pub self_samples: u64,
    pub self_pct: f64,
}

/// Visual annotation for one row, derived from the `incl_pct` /
/// `self_pct` ratio + label shape. Helps the eye distinguish
/// "actual hot work" from "structural call-chain frame".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// Pure leaf: self ≈ incl. The CPU was here when the sample
    /// fired the majority of the time it was on the stack.
    LeafHot,
    /// Mostly leaf with some passes-through. Self covers more
    /// than half of inclusive.
    Hot,
    /// Frame is on many stacks but rarely the leaf. Typical
    /// dispatcher / wrapper.
    Wrapper,
    /// Frame is at or near the very top of every stack but does no
    /// own work. `_start` / `FnOnce::call_once` / `<binary>::main`.
    Trampoline,
    /// Hex-address label that addr2line + dynamic-symbol fallback
    /// couldn't resolve. Carries the library origin if the
    /// renderer attached it.
    Unresolved,
}

impl RowKind {
    /// Two-letter ASCII tag for the rendered table. Easy to scan
    /// vertically: `LH` (leaf hot) / `HW` (hot work) / `WR`
    /// (wrapper) / `EP` (entry / trampoline) / `??` (unresolved).
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
    if incl_pct >= 95.0 && self_pct < 1.0 {
        return RowKind::Trampoline;
    }
    if incl_pct == 0.0 {
        return RowKind::LeafHot; // shouldn't happen but defensive
    }
    let leaf_ratio = self_pct / incl_pct;
    if leaf_ratio >= 0.9 {
        RowKind::LeafHot
    } else if leaf_ratio >= 0.5 {
        RowKind::Hot
    } else {
        RowKind::Wrapper
    }
}

/// Top-N hot leaf frames. `n` rows, sorted descending by sample
/// count. For leaf-mode `incl_*` and `self_*` columns are equal —
/// every leaf-frame sample contributes to both.
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

/// Top-N inclusive: each function gets a sample if it appears
/// anywhere on a sampled stack.
///
/// The report ALSO carries each function's leaf-frame count as
/// `self_*`, so the user can spot trampolines (high incl %, ~0 %
/// self) at a glance without losing them from the table.
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

fn pct_of(samples: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (samples as f64 / total as f64) * 100.0
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
                    width: 4,
                },
                Column {
                    header: "kind".into(),
                    align: Align::Left,
                    width: 4,
                },
                Column {
                    header: "incl %".into(),
                    align: Align::Right,
                    width: 7,
                },
                Column {
                    header: "self %".into(),
                    align: Align::Right,
                    width: 7,
                },
                Column {
                    header: "samples".into(),
                    align: Align::Right,
                    width: 8,
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
        // Append a tiny legend so the kind tags are self-explanatory
        // without the user having to consult the docstring.
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
