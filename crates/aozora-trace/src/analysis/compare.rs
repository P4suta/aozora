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

#[derive(Debug, Clone)]
pub struct ComparisonReport {
    pub before_total: u64,
    pub after_total: u64,
    pub rows: Vec<ComparisonRow>,
}

#[derive(Debug, Clone)]
pub struct ComparisonRow {
    pub label: String,
    pub before_pct: f64,
    pub after_pct: f64,
    pub delta_pct: f64,
    pub status: ChangeStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeStatus {
    Shifted,
    Appeared,
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
