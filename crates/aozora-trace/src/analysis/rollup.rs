//! Category rollup — group function names into named buckets.
//!
//! `hot_leaves` shows individual functions; `rollup` shows
//! *categories of functions* (Phase 1 / Phase 3 / `corpus_load` /
//! allocation / etc.). Drives the high-level "where is the time
//! conceptually going?" view in PROFILING.md.
//!
//! Categorisation is driven by [`crate::Categorizer`]; pass either
//! [`crate::RollupConfig::aozora_defaults`] or a user TOML.

use std::collections::HashMap;

use crate::render::{Align, Column, TableBuilder};
use crate::{Categorizer, TableRenderable, Trace};

#[derive(Debug, Clone)]
pub struct RollupReport {
    pub total_samples: u64,
    pub rows: Vec<RollupRow>,
}

#[derive(Debug, Clone)]
pub struct RollupRow {
    pub category: String,
    pub samples: u64,
    pub pct: f64,
    /// How many distinct functions matched this category. Useful
    /// sanity check: a category with `distinct_funcs = 1` is
    /// suspicious (probably misnamed).
    pub distinct_funcs: usize,
}

/// Bucket every leaf-frame sample into a category. Categories with
/// zero samples are still emitted (in declaration order) so the
/// report row order is stable across traces.
#[must_use]
pub fn rollup(trace: &Trace, categorizer: &Categorizer) -> RollupReport {
    // (category_name, function_name) → sample count.
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
    // Build rows in declaration order, then trailing `unknown`,
    // then any extras alphabetically.
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
                (samples as f64 / total as f64) * 100.0
            },
            distinct_funcs: funcs.len(),
        });
    }
    // Anything left over (incl. unknown) — append at the end,
    // sorted by sample count descending so it eyeballs cleanly.
    let mut leftover: Vec<(String, HashMap<String, u64>)> = grouped.into_iter().collect();
    leftover.sort_by_key(|(_, funcs)| std::cmp::Reverse(funcs.values().sum::<u64>()));
    for (cat, funcs) in leftover {
        let samples: u64 = funcs.values().sum();
        rows.push(RollupRow {
            category: cat,
            samples,
            pct: if total == 0 {
                0.0
            } else {
                (samples as f64 / total as f64) * 100.0
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
                    width: 6,
                },
                Column {
                    header: "samples".into(),
                    align: Align::Right,
                    width: 8,
                },
                Column {
                    header: "funcs".into(),
                    align: Align::Right,
                    width: 5,
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
