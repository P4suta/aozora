//! Top-N hot frames — leaf or inclusive.
//!
//! - **Leaf** (`hot_leaves`): per-sample, count *only* the leaf-most
//!   frame. Tells you "where the CPU is *literally* spending time
//!   right now". Best for: identifying tight loops, victim functions.
//! - **Inclusive** (`hot_inclusive`): per-sample, count *every*
//!   frame on the call stack (deduplicated per sample to avoid
//!   double-counting recursion). Tells you "which functions are on
//!   the hot path". Best for: identifying entry points to optimise.

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
    pub samples: u64,
    pub pct: f64,
}

/// Top-N hot leaf frames. `n` rows, sorted descending by sample
/// count.
#[must_use]
pub fn hot_leaves(trace: &Trace, n: usize) -> HotReport {
    let mut counts: HashMap<String, u64> = HashMap::new();
    let mut total = 0u64;
    for thread in &trace.threads {
        for sample in &thread.samples {
            let Some(stack_idx) = sample.stack_idx else {
                continue;
            };
            let Some(entry) = thread.stack_table.get(stack_idx) else {
                continue;
            };
            let label = thread.frame_label(entry.frame_idx);
            *counts.entry(label).or_insert(0) += sample.weight;
            total += sample.weight;
        }
    }
    HotReport {
        mode: HotMode::Leaf,
        total_samples: total,
        rows: top_n_rows(counts, total, n),
    }
}

/// Top-N inclusive: each function gets a sample if it appears
/// anywhere on a sampled stack. Per-sample dedup keeps recursive
/// functions honest (one sample, one count).
#[must_use]
pub fn hot_inclusive(trace: &Trace, n: usize) -> HotReport {
    let mut counts: HashMap<String, u64> = HashMap::new();
    let mut total = 0u64;
    let mut seen: HashSet<usize> = HashSet::new();
    for thread in &trace.threads {
        for sample in &thread.samples {
            total += sample.weight;
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
    HotReport {
        mode: HotMode::Inclusive,
        total_samples: total,
        rows: top_n_rows(counts, total, n),
    }
}

fn top_n_rows(counts: HashMap<String, u64>, total: u64, n: usize) -> Vec<HotRow> {
    let mut rows: Vec<HotRow> = counts
        .into_iter()
        .map(|(label, samples)| HotRow {
            pct: if total == 0 {
                0.0
            } else {
                (samples as f64 / total as f64) * 100.0
            },
            label,
            samples,
        })
        .collect();
    rows.sort_by(|a, b| {
        b.samples
            .cmp(&a.samples)
            .then_with(|| a.label.cmp(&b.label))
    });
    rows.truncate(n);
    rows
}

impl TableRenderable for HotReport {
    fn render_table(&self) -> String {
        let title = match self.mode {
            HotMode::Leaf => format!(
                "Top {} HOT LEAF frames ({} total samples)",
                self.rows.len(),
                self.total_samples
            ),
            HotMode::Inclusive => format!(
                "Top {} HOT INCLUSIVE frames ({} total samples)",
                self.rows.len(),
                self.total_samples
            ),
        };
        let mut t = TableBuilder::new(
            title,
            vec![
                Column {
                    header: "rank".into(),
                    align: Align::Right,
                    width: 4,
                },
                Column {
                    header: "%".into(),
                    align: Align::Right,
                    width: 6,
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
                format!("{:.2}", r.pct),
                format!("{}", r.samples),
                r.label.clone(),
            ]);
        }
        t.render()
    }
}
