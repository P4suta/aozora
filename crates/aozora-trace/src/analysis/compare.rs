//! Diff two traces (before vs after).
//!
//! Aggregates leaf-frame samples on both, normalises to per-trace
//! percentages (raw counts depend on trace duration / sample rate),
//! then surfaces:
//!
//! - **shifted**: function present in both, percentage moved
//! - **appeared**: present in `after` but not `before`
//! - **disappeared**: present in `before` but not `after`
//!
//! Sorted by absolute percentage delta, descending. Drives the
//! "what did my optimisation actually change?" question.

use std::collections::{BTreeSet, HashMap};

use crate::analysis::hot::hot_leaves;
use crate::render::{Align, Column, TableBuilder};
use crate::{TableRenderable, Trace};

/// Result of diffing two traces, produced by [`compare`].
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonReport {
    /// Total samples in the `before` trace (for context in the header).
    pub before_total: u64,
    /// Total samples in the `after` trace.
    pub after_total: u64,
    /// Per-function diff rows, sorted by descending absolute |Δ%|.
    pub rows: Vec<ComparisonRow>,
}

/// One function's before/after comparison.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonRow {
    /// Function (leaf-frame) label.
    pub label: String,
    /// Its leaf-frame share of the `before` trace, in percent.
    pub before_pct: f64,
    /// Its leaf-frame share of the `after` trace, in percent.
    pub after_pct: f64,
    /// `after_pct - before_pct`. Positive means the function got hotter.
    pub delta_pct: f64,
    /// Whether the function shifted, appeared, or disappeared.
    pub status: ChangeStatus,
}

/// How a function's presence changed between the two traces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    /// Present in both traces; its percentage moved (possibly by zero).
    Shifted,
    /// Present in `after` but not `before` (newly hot).
    Appeared,
    /// Present in `before` but not `after` (no longer sampled).
    Disappeared,
}

/// Compare two traces. `top` rows by absolute |Δ%| (per-side
/// limit, pre-merge: we ask each side for `top * 4` so we have
/// enough union coverage).
#[must_use]
pub fn compare(before: &Trace, after: &Trace, top: usize) -> ComparisonReport {
    let bsize = top.saturating_mul(4).max(top);
    let b = hot_leaves(before, bsize);
    let a = hot_leaves(after, bsize);
    // For comparison purposes the leaf-mode `self_pct` IS the
    // canonical "where the CPU was" percentage (incl_pct equals
    // self_pct in leaf mode).
    let bmap: HashMap<&str, f64> = b
        .rows
        .iter()
        .map(|r| (r.label.as_str(), r.self_pct))
        .collect();
    let amap: HashMap<&str, f64> = a
        .rows
        .iter()
        .map(|r| (r.label.as_str(), r.self_pct))
        .collect();

    let mut rows: Vec<ComparisonRow> = Vec::new();
    let mut keys: BTreeSet<&str> = BTreeSet::new();
    keys.extend(bmap.keys());
    keys.extend(amap.keys());

    for key in keys {
        let bp = bmap.get(key).copied().unwrap_or(0.0);
        let ap = amap.get(key).copied().unwrap_or(0.0);
        let delta = ap - bp;
        let status = match (bp > 0.0, ap > 0.0) {
            (true, true) => ChangeStatus::Shifted,
            (false, true) => ChangeStatus::Appeared,
            (true, false) => ChangeStatus::Disappeared,
            (false, false) => continue,
        };
        rows.push(ComparisonRow {
            label: key.to_owned(),
            before_pct: bp,
            after_pct: ap,
            delta_pct: delta,
            status,
        });
    }
    rows.sort_by(|a, b| {
        b.delta_pct
            .abs()
            .partial_cmp(&a.delta_pct.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows.truncate(top);

    ComparisonReport {
        before_total: b.total_samples,
        after_total: a.total_samples,
        rows,
    }
}

impl TableRenderable for ComparisonReport {
    fn render_table(&self) -> String {
        let title = format!(
            "Trace comparison ({} → {} total samples)",
            self.before_total, self.after_total
        );
        let mut t = TableBuilder::new(
            title,
            vec![
                Column {
                    header: "before %".into(),
                    align: Align::Right,
                    width: 8,
                },
                Column {
                    header: "after %".into(),
                    align: Align::Right,
                    width: 8,
                },
                Column {
                    header: "Δ".into(),
                    align: Align::Right,
                    width: 8,
                },
                Column {
                    header: "status".into(),
                    align: Align::Left,
                    width: 12,
                },
                Column {
                    header: "function".into(),
                    align: Align::Left,
                    width: 0,
                },
            ],
        );
        for r in &self.rows {
            let status = match r.status {
                ChangeStatus::Shifted => "shifted",
                ChangeStatus::Appeared => "appeared",
                ChangeStatus::Disappeared => "disappeared",
            };
            let delta = if r.delta_pct >= 0.0 {
                format!("+{:.2}", r.delta_pct)
            } else {
                format!("{:.2}", r.delta_pct)
            };
            t.push_row(vec![
                format!("{:.2}", r.before_pct),
                format!("{:.2}", r.after_pct),
                delta,
                status.into(),
                r.label.clone(),
            ]);
        }
        t.render()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::{Value, json};

    use super::{ChangeStatus, compare};
    use crate::Trace;

    /// Build a single-thread trace whose leaves are `(name, weight)`
    /// pairs, each its own root stack. Keeps the `ChangeStatus` arms
    /// assertable in-crate (the enum is not re-exported).
    fn leaves(samples: &[(&str, u64)]) -> Trace {
        let strings: Vec<Value> = samples.iter().map(|(n, _)| json!(n)).collect();
        let func_name: Vec<Value> = (0..samples.len()).map(|i| json!(i)).collect();
        let func_res: Vec<Value> = (0..samples.len()).map(|_| json!(0)).collect();
        let frame_addr: Vec<Value> = (0..samples.len()).map(|i| json!(i + 1)).collect();
        let frame_func: Vec<Value> = (0..samples.len()).map(|i| json!(i)).collect();
        let stack_prefix: Vec<Value> = (0..samples.len()).map(|_| json!(null)).collect();
        let stack_frame: Vec<Value> = (0..samples.len()).map(|i| json!(i)).collect();
        let sample_stack: Vec<Value> = (0..samples.len()).map(|i| json!(i)).collect();
        let sample_weight: Vec<Value> = samples.iter().map(|(_, w)| json!(w)).collect();
        let json = json!({
            "libs": [{ "name": "bin", "path": "/bin" }],
            "threads": [{
                "name": "t",
                "stringArray": strings,
                "stackTable": { "prefix": stack_prefix, "frame": stack_frame },
                "frameTable": { "address": frame_addr, "func": frame_func },
                "funcTable": { "name": func_name, "resource": func_res },
                "resourceTable": { "lib": [0] },
                "samples": { "stack": sample_stack, "weight": sample_weight },
            }],
        });
        Trace::from_json(&json, PathBuf::from("t")).expect("load")
    }

    #[test]
    fn status_variants_are_assigned_correctly() {
        // f: both sides → Shifted; g: before-only → Disappeared;
        // h: after-only → Appeared.
        let before = leaves(&[("f", 8), ("g", 2)]);
        let after = leaves(&[("f", 2), ("h", 8)]);
        let report = compare(&before, &after, 10);
        let status_of = |label: &str| {
            report
                .rows
                .iter()
                .find(|r| r.label == label)
                .map(|r| r.status)
        };
        assert_eq!(
            status_of("f"),
            Some(ChangeStatus::Shifted),
            "present in both ⇒ Shifted"
        );
        assert_eq!(
            status_of("g"),
            Some(ChangeStatus::Disappeared),
            "before-only ⇒ Disappeared"
        );
        assert_eq!(
            status_of("h"),
            Some(ChangeStatus::Appeared),
            "after-only ⇒ Appeared"
        );
    }

    #[test]
    fn unchanged_function_is_shifted_with_zero_delta() {
        let t = leaves(&[("f", 1)]);
        let report = compare(&t, &t, 10);
        let f = report.rows.iter().find(|r| r.label == "f").expect("f");
        assert_eq!(f.status, ChangeStatus::Shifted, "in both ⇒ Shifted");
        assert!(f.delta_pct.abs() < 1e-9, "identical ⇒ zero delta");
    }
}
