//! Groups leaf-frame samples into configured categories through [`rollup`].

use std::cmp::Reverse;
use std::collections::HashMap;

use crate::render::{Align, Column, TableBuilder};
use crate::{Categorizer, TableRenderable, Trace};

const PERCENT_SCALE: f64 = 100.0;
const PERCENT_COLUMN_WIDTH: usize = 6;
const SAMPLES_COLUMN_WIDTH: usize = 8;
const FUNCTIONS_COLUMN_WIDTH: usize = 5;

/// Distribution of samples by category, produced by [`rollup`].
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RollupReport {
    /// Total weight of samples across the trace.
    pub total_samples: u64,
    /// Configured categories in declaration order, followed by remaining categories in descending weight of samples.
    pub rows: Vec<RollupRow>,
}

/// One category's aggregated samples.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RollupRow {
    /// Configured category label, or `unknown` for unmatched functions.
    pub category: String,
    /// Weight of leaf-frame samples in this category.
    pub samples: u64,
    /// `samples` as a percentage of [`RollupReport::total_samples`].
    pub pct: f64,
    /// Number of distinct function labels assigned to this category.
    pub distinct_funcs: usize,
}

/// Groups leaf-frame samples by category, including configured categories with zero samples.
#[expect(
    clippy::cast_precision_loss,
    reason = "sample shares are rendered as approximate percentages"
)]
#[must_use]
pub fn rollup(trace: &Trace, categorizer: &Categorizer) -> RollupReport {
    let mut grouped: HashMap<String, HashMap<String, u64>> = HashMap::new();
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
            let cat = categorizer.classify(&label).to_owned();
            *grouped.entry(cat).or_default().entry(label).or_insert(0) += sample.weight;
        }
    }
    let mut rows: Vec<RollupRow> = Vec::new();
    for cat_name in categorizer.category_names() {
        let funcs = grouped.remove(cat_name).unwrap_or_default();
        let samples: u64 = funcs.values().sum();
        rows.push(RollupRow {
            category: cat_name.to_owned(),
            samples,
            pct: if total == 0 {
                0.0
            } else {
                (samples as f64 / total as f64) * PERCENT_SCALE
            },
            distinct_funcs: funcs.len(),
        });
    }
    let mut leftover: Vec<(String, HashMap<String, u64>)> = grouped.into_iter().collect();
    leftover.sort_by_key(|(_, funcs)| Reverse(funcs.values().sum::<u64>()));
    for (cat, funcs) in leftover {
        let samples: u64 = funcs.values().sum();
        rows.push(RollupRow {
            category: cat,
            samples,
            pct: if total == 0 {
                0.0
            } else {
                (samples as f64 / total as f64) * PERCENT_SCALE
            },
            distinct_funcs: funcs.len(),
        });
    }
    RollupReport {
        total_samples: total,
        rows,
    }
}

impl TableRenderable for RollupReport {
    fn render_table(&self) -> String {
        let title = format!("Category rollup ({} total samples)", self.total_samples);
        let mut t = TableBuilder::new(
            title,
            vec![
                Column {
                    header: "%".into(),
                    align: Align::Right,
                    width: PERCENT_COLUMN_WIDTH,
                },
                Column {
                    header: "samples".into(),
                    align: Align::Right,
                    width: SAMPLES_COLUMN_WIDTH,
                },
                Column {
                    header: "funcs".into(),
                    align: Align::Right,
                    width: FUNCTIONS_COLUMN_WIDTH,
                },
                Column {
                    header: "category".into(),
                    align: Align::Left,
                    width: 0,
                },
            ],
        );
        for r in &self.rows {
            t.push_row(vec![
                format!("{:.2}", r.pct),
                format!("{}", r.samples),
                format!("{}", r.distinct_funcs),
                r.category.clone(),
            ]);
        }
        t.render()
    }
}
