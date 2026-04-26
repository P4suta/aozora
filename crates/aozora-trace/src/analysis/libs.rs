//! Library-level distribution: where samples landed by loaded
//! object (binary / libc / vdso / …).
//!
//! A 30-second sanity check: "is my parser actually hot, or is the
//! trace dominated by glibc memmove?" If `libc.so.6` is the top
//! row, `cargo bench` is measuring memcpy bandwidth, not your code.

use std::collections::HashMap;

use crate::render::{Align, Column, TableBuilder};
use crate::{TableRenderable, Trace};

#[derive(Debug, Clone)]
pub struct LibraryReport {
    pub total_samples: u64,
    pub rows: Vec<LibraryRow>,
}

#[derive(Debug, Clone)]
pub struct LibraryRow {
    pub library: String,
    pub samples: u64,
    pub pct: f64,
}

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
                (samples as f64 / total as f64) * 100.0
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
                    width: 6,
                },
                Column {
                    header: "samples".into(),
                    align: Align::Right,
                    width: 8,
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
