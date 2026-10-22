use std::collections::HashMap;

use crate::Trace;

/// One row in the folded output.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FoldedStack {
    /// Frame labels in root-to-leaf order, ready to join with `;`.
    pub stack: Vec<String>,
    /// Total sample weight that landed on this exact stack.
    pub samples: u64,
}

/// Aggregate every (thread, sample) into a folded-stack list.
///
/// Identical stacks are summed and sorted by sample count descending.
#[must_use]
pub fn folded_stacks(trace: &Trace) -> Vec<FoldedStack> {
    let mut grouped: HashMap<Vec<String>, u64> = HashMap::new();
    for thread in &trace.threads {
        for sample in &thread.samples {
            let Some(stack_idx) = sample.stack_idx else {
                continue;
            };
            let mut frames: Vec<String> = thread
                .walk_stack(stack_idx)
                .map(|fi| thread.frame_label(fi))
                .collect();
            frames.reverse();
            *grouped.entry(frames).or_insert(0) += sample.weight;
        }
    }
    let mut rows: Vec<FoldedStack> = grouped
        .into_iter()
        .map(|(stack, samples)| FoldedStack { stack, samples })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.samples));
    rows
}

/// Format folded stacks for direct `flamegraph.pl` consumption:
/// `root;parent;…;leaf <samples>\n` per line.
#[must_use]
pub fn render_folded(rows: &[FoldedStack]) -> String {
    let mut out = String::new();
    for r in rows {
        for (i, frame) in r.stack.iter().enumerate() {
            if i > 0 {
                out.push(';');
            }
            for ch in frame.chars() {
                if ch == ';' {
                    out.push(':');
                } else {
                    out.push(ch);
                }
            }
        }
        out.push(' ');
        out.push_str(&r.samples.to_string());
        out.push('\n');
    }
    out
}
