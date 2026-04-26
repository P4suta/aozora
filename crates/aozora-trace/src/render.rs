//! Plain-text table rendering for analysis reports.
//!
//! Each analysis returns a typed report struct (e.g. [`crate::analysis::HotReport`]).
//! Reports implement [`TableRenderable`] so the CLI can pretty-print
//! them uniformly. We deliberately don't pull in `comfy-table` /
//! `tabled` etc. — the format is fixed-width 3-column "rank / pct /
//! label" and rolling our own keeps the dep graph trivial.

/// Implemented by every analysis report. Render to stdout-friendly
/// text or to a structured serializable form (planned).
pub trait TableRenderable {
    fn render_table(&self) -> String;
}

pub(crate) struct TableBuilder {
    title: String,
    rows: Vec<Row>,
    columns: Vec<Column>,
}

pub(crate) struct Column {
    pub header: String,
    pub align: Align,
    pub width: usize,
}

pub(crate) enum Align {
    Left,
    Right,
}

pub(crate) struct Row {
    pub cells: Vec<String>,
}

impl TableBuilder {
    pub(crate) fn new(title: impl Into<String>, columns: Vec<Column>) -> Self {
        Self {
            title: title.into(),
            rows: Vec::new(),
            columns,
        }
    }

    pub(crate) fn push_row(&mut self, cells: Vec<String>) {
        self.rows.push(Row { cells });
    }

    pub(crate) fn render(self) -> String {
        let mut out = String::new();
        if !self.title.is_empty() {
            out.push_str(&self.title);
            out.push('\n');
            out.push_str(&"=".repeat(self.title.chars().count()));
            out.push('\n');
        }
        // Compute final column widths: max of header / declared / actual cell widths.
        let mut widths: Vec<usize> = self
            .columns
            .iter()
            .map(|c| c.width.max(c.header.chars().count()))
            .collect();
        for row in &self.rows {
            for (i, cell) in row.cells.iter().enumerate() {
                if i < widths.len() {
                    widths[i] = widths[i].max(cell.chars().count());
                }
            }
        }
        // Header row.
        for (i, c) in self.columns.iter().enumerate() {
            push_padded(&mut out, &c.header, widths[i], &c.align);
            out.push_str("  ");
        }
        out.push('\n');
        // Separator.
        for &w in &widths {
            out.push_str(&"-".repeat(w));
            out.push_str("  ");
        }
        out.push('\n');
        // Body.
        for row in &self.rows {
            for (i, cell) in row.cells.iter().enumerate() {
                let align = self.columns.get(i).map_or(&Align::Left, |c| &c.align);
                push_padded(&mut out, cell, widths[i], align);
                out.push_str("  ");
            }
            out.push('\n');
        }
        out
    }
}

fn push_padded(out: &mut String, s: &str, width: usize, align: &Align) {
    let len = s.chars().count();
    let pad = width.saturating_sub(len);
    match align {
        Align::Left => {
            out.push_str(s);
            out.push_str(&" ".repeat(pad));
        }
        Align::Right => {
            out.push_str(&" ".repeat(pad));
            out.push_str(s);
        }
    }
}
