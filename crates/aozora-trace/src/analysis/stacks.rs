//! Print full call stacks containing functions matching a regex.
//!
//! Use case: "I see `aho_corasick::packed::teddy::*` in the hot
//! list — *who's calling it*?". A `--filter teddy` query returns
//! the top-K distinct call stacks that include the matching frame
//! anywhere in the chain.

use std::collections::HashMap;

use regex::Regex;

use crate::render::{Align, Column, TableBuilder};
use crate::{TableRenderable, Trace};

/// One distinct call stack matching the filter.
#[derive(Debug, Clone)]
pub struct MatchedStack {
    /// Frame labels, leaf first.
    pub frames: Vec<String>,
    /// How many samples landed on this exact stack.
    pub samples: u64,
    /// Percentage of total samples.
    pub pct: f64,
}

#[derive(Debug, Clone)]
pub struct MatchedStacksReport {
    pub filter: String,
    pub total_samples: u64,
    pub matched_samples: u64,
    pub stacks: Vec<MatchedStack>,
}

/// Find call stacks where any frame matches `regex`. Aggregates
/// identical stacks and returns the top `limit` by sample count.
#[must_use]
pub fn matching_stacks(trace: &Trace, regex: &Regex, limit: usize) -> MatchedStacksReport {
    let mut grouped: HashMap<Vec<String>, u64> = HashMap::new();
    let mut total = 0u64;
    let mut matched = 0u64;
    for thread in &trace.threads {
        for sample in &thread.samples {
            total += sample.weight;
            let Some(stack_idx) = sample.stack_idx else {
                continue;
            };
            let frames: Vec<String> = thread
                .walk_stack(stack_idx)
                .map(|fi| thread.frame_label(fi))
                .collect();
            if frames.iter().any(|f| regex.is_match(f)) {
                matched += sample.weight;
                *grouped.entry(frames).or_insert(0) += sample.weight;
            }
        }
    }
    let mut stacks: Vec<MatchedStack> = grouped
        .into_iter()
        .map(|(frames, samples)| MatchedStack {
            frames,
            samples,
            pct: if total == 0 {
                0.0
            } else {
                (samples as f64 / total as f64) * 100.0
            },
        })
        .collect();
    stacks.sort_by_key(|s| std::cmp::Reverse(s.samples));
    stacks.truncate(limit);
    MatchedStacksReport {
        filter: regex.as_str().to_owned(),
        total_samples: total,
        matched_samples: matched,
        stacks,
    }
}

impl TableRenderable for MatchedStacksReport {
    fn render_table(&self) -> String {
        let mut out = String::new();
        let title = format!(
            "Call stacks matching `{}` — {} matched samples / {} total ({:.2}%)",
            self.filter,
            self.matched_samples,
            self.total_samples,
            if self.total_samples == 0 {
                0.0
            } else {
                self.matched_samples as f64 / self.total_samples as f64 * 100.0
            }
        );
        out.push_str(&title);
        out.push('\n');
        out.push_str(&"=".repeat(title.chars().count()));
        out.push('\n');
        for (i, st) in self.stacks.iter().enumerate() {
            let mut t = TableBuilder::new(
                format!("#{} — {} samples ({:.2}%)", i + 1, st.samples, st.pct),
                vec![
                    Column {
                        header: "depth".into(),
                        align: Align::Right,
                        width: 5,
                    },
                    Column {
                        header: "frame".into(),
                        align: Align::Left,
                        width: 0,
                    },
                ],
            );
            for (depth, label) in st.frames.iter().enumerate() {
                t.push_row(vec![format!("{}", depth), label.clone()]);
            }
            out.push_str(&t.render());
            out.push('\n');
        }
        out
    }
}
