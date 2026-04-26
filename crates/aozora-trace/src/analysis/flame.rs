//! Folded-stack format — feeds `flamegraph.pl` / inferno.
//!
//! Each line is `root;parent;…;leaf <samples>`.
//!
//! Brendan Gregg's folded-stack convention; both `flamegraph.pl` and
//! `inferno-flamegraph` consume it directly, so we don't need to bake
//! a flamegraph renderer into this crate.
//!
//! ## Why root-first
//!
//! `walk_stack` yields leaf-first.
//!
//! Folded-stack format expects root-first, so we reverse the walk
//! before joining; the join uses `;` (the format's required
//! separator).

use std::collections::HashMap;

use crate::Trace;

/// One row in the folded output.
#[derive(Debug, Clone)]
pub struct FoldedStack {
    pub stack: Vec<String>,
    pub samples: u64,
}

/// Aggregate every (thread, sample) into a folded-stack list.
///
/// Lines are deduplicated (identical stacks summed) and sorted by
/// sample count descending — the inferno renderer doesn't care
/// about order, but the descending sort makes the output greppable.
#[must_use]
pub fn folded_stacks(trace: &Trace) -> Vec<FoldedStack> {
    let mut grouped: HashMap<Vec<String>, u64> = HashMap::new();
    for thread in &trace.threads {
        for sample in &thread.samples {
            let Some(stack_idx) = sample.stack_idx else {
                continue;
            };
            // Walk leaf->root, then reverse so the stored stack is
            // root->leaf to match folded-stack convention.
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
        // `;` is folded-stack's separator. Sanitise function names
        // that happen to contain `;` (rare in Rust mangled names,
        // but possible in raw addresses) by replacing with `:`.
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
