//! Reports the shares of samples by loaded library through [`library_distribution`].

use std::collections::HashMap;

use crate::render::{Align, Column, TableBuilder};
use crate::{TableRenderable, Trace};

const PERCENT_SCALE: f64 = 100.0;
const PERCENT_COLUMN_WIDTH: usize = 6;
const SAMPLES_COLUMN_WIDTH: usize = 8;

/// Distribution of samples by library, produced by [`library_distribution`].
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryReport {
    /// Total weight of samples across the trace.
    pub total_samples: u64,
    /// Libraries sorted by descending weight of samples.
    pub rows: Vec<LibraryRow>,
}

/// One library's share of the samples.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryRow {
    /// Library name, or `(unattributed)` for samples whose leaf frame maps to no library.
    pub library: String,
    /// Weight of leaf-frame samples attributed to this library.
    pub samples: u64,
    /// `samples` as a percentage of [`LibraryReport::total_samples`].
    pub pct: f64,
}

/// Returns library shares sorted by descending weight of leaf-frame samples.
/// Groups samples without a stack or library under `(unattributed)`.
#[expect(
    clippy::cast_precision_loss,
    reason = "sample shares are rendered as approximate percentages"
)]
#[must_use]
pub fn library_distribution(trace: &Trace) -> LibraryReport {
    let mut counts: HashMap<String, u64> = HashMap::new();
    let mut total = 0u64;
    let unattributed = "(unattributed)";
    for thread in &trace.threads {
        for sample in &thread.samples {
            total += sample.weight;
            let Some(stack_idx) = sample.stack_idx else {
                *counts.entry(unattributed.to_owned()).or_insert(0) += sample.weight;
                continue;
            };
            let Some(entry) = thread.stack_table.get(stack_idx) else {
                *counts.entry(unattributed.to_owned()).or_insert(0) += sample.weight;
                continue;
            };
            let lib_name = thread
                .frame_library(entry.frame_idx)
                .and_then(|i| trace.libs.get(i))
                .map_or_else(|| unattributed.to_owned(), |l| l.name.clone());
            *counts.entry(lib_name).or_insert(0) += sample.weight;
        }
    }
    let mut rows: Vec<LibraryRow> = counts
        .into_iter()
        .map(|(library, samples)| LibraryRow {
            pct: if total == 0 {
                0.0
            } else {
                (samples as f64 / total as f64) * PERCENT_SCALE
            },
            library,
            samples,
        })
        .collect();
    rows.sort_by(|a, b| {
        b.samples
            .cmp(&a.samples)
            .then_with(|| a.library.cmp(&b.library))
    });
    LibraryReport {
        total_samples: total,
        rows,
    }
}

impl TableRenderable for LibraryReport {
    fn render_table(&self) -> String {
        let title = format!(
            "Library distribution ({} total samples)",
            self.total_samples
        );
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
                    header: "library".into(),
                    align: Align::Left,
                    width: 0,
                },
            ],
        );
        for r in &self.rows {
            t.push_row(vec![
                format!("{:.2}", r.pct),
                format!("{}", r.samples),
                r.library.clone(),
            ]);
        }
        t.render()
    }
}
