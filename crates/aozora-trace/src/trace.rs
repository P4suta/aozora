//! Core trace data model.
//!
//! Mirrors the parts of the gecko-profile JSON we actually need.
//! Internal tables (`stack_table`, `frame_table`, `func_table`,
//! `string_array`, `resource_table`) keep their on-disk shape so
//! reconstructing call stacks is a simple index walk.
//!
//! Design rule: this module exposes *data*, not *analyses*. Every
//! aggregation, percentage, sort, or report lives under
//! [`crate::analysis`]. The accessors here are the minimal toolkit
//! those analyses share — frame label, frame library, stack walk.

use std::path::PathBuf;

/// A loaded samply trace.
#[derive(Debug, Clone)]
pub struct Trace {
    pub libs: Vec<Library>,
    pub threads: Vec<Thread>,
    /// Source file the trace was loaded from. Drives sidecar-cache
    /// path resolution and shows up in diagnostic prints.
    pub source_path: PathBuf,
}

/// A loaded library (ELF file, vdso, or anonymous mapping).
#[derive(Debug, Clone)]
pub struct Library {
    pub name: String,
    pub path: String,
    pub debug_path: String,
    /// Breakpad-style debug id (UPPER hex + age). Often empty in
    /// samply-recorded traces — `code_id` is the field that actually
    /// matches the binary's `gnu-build-id`.
    pub debug_id: String,
    /// Hex-encoded gnu-build-id of the loaded ELF. The
    /// load-time identity check uses this; matched against the
    /// binary's `.note.gnu.build-id` by [`crate::Symbolicator::verify_against`].
    pub code_id: String,
}

/// One profiled thread with its sample stream + stack tables.
#[derive(Debug, Clone)]
pub struct Thread {
    pub tid: i64,
    pub name: String,
    pub is_main: bool,
    pub samples: Vec<Sample>,

    // Internal tables — exposed `pub` because the analysis modules
    // consult them directly. Mutating from outside the crate is
    // unsupported (no invariant checking).
    pub string_array: Vec<String>,
    pub stack_table: Vec<StackEntry>,
    pub frame_table: Vec<FrameRow>,
    pub func_table: Vec<FuncRow>,
    pub resource_table: Vec<ResourceRow>,
    /// Resolved function names per frame index — `None` until
    /// [`crate::Symbolicator`] fills them in. Same length as
    /// `frame_table`.
    pub resolved: Vec<Option<String>>,
}

/// A single CPU sample.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub time_ms: f64,
    pub stack_idx: Option<usize>,
    pub weight: u64,
}

/// A node in the stack-table tree. `prefix.prefix.prefix...` chain
/// terminates at `None` (the root).
#[derive(Debug, Clone, Copy)]
pub struct StackEntry {
    pub prefix: Option<usize>,
    pub frame_idx: usize,
}

/// One frame: a (PC, func-table-row) pair.
#[derive(Debug, Clone, Copy)]
pub struct FrameRow {
    pub address: u64,
    pub func_idx: usize,
}

/// One source-language function: its name + owning resource.
#[derive(Debug, Clone, Copy)]
pub struct FuncRow {
    /// Index into [`Thread::string_array`]. May point at an empty
    /// string for un-symbolicated traces.
    pub name_idx: usize,
    pub resource_idx: Option<usize>,
}

/// One resource: maps a function back to its loaded library.
#[derive(Debug, Clone, Copy)]
pub struct ResourceRow {
    pub lib_idx: Option<usize>,
}

impl Thread {
    /// Best-known label for `frame_idx`. Prefers the symbolicated
    /// cache, falls back to the raw `funcTable` string, finally to
    /// the hex address.
    #[must_use]
    pub fn frame_label(&self, frame_idx: usize) -> String {
        if let Some(Some(label)) = self.resolved.get(frame_idx) {
            return label.clone();
        }
        let frame = self.frame_table[frame_idx];
        let func = self.func_table[frame.func_idx];
        let raw = self
            .string_array
            .get(func.name_idx)
            .map_or("", String::as_str);
        if raw.is_empty() {
            format!("0x{:x}", frame.address)
        } else {
            raw.to_owned()
        }
    }

    /// Library index backing `frame_idx`, or `None` if unattributed.
    #[must_use]
    pub fn frame_library(&self, frame_idx: usize) -> Option<usize> {
        let frame = self.frame_table[frame_idx];
        let func = self.func_table[frame.func_idx];
        let resource = self.resource_table[func.resource_idx?];
        resource.lib_idx
    }

    /// Address recorded for `frame_idx`. Useful for symbolicator
    /// input.
    #[must_use]
    pub fn frame_address(&self, frame_idx: usize) -> u64 {
        self.frame_table[frame_idx].address
    }

    /// Iterate the stack from leaf-most frame to root, yielding
    /// frame indices. Cycles in the prefix chain (which gecko
    /// schema forbids but we don't enforce) would loop indefinitely
    /// — bounded internally by the stack-table length.
    #[must_use]
    pub fn walk_stack(&self, leaf_stack: usize) -> StackWalker<'_> {
        StackWalker {
            thread: self,
            cursor: Some(leaf_stack),
            steps_remaining: self.stack_table.len(),
        }
    }
}

/// Iterator returned by [`Thread::walk_stack`]. Bounded by stack-
/// table length to defend against pathological prefix cycles.
#[derive(Debug)]
pub struct StackWalker<'t> {
    thread: &'t Thread,
    cursor: Option<usize>,
    steps_remaining: usize,
}

impl Iterator for StackWalker<'_> {
    type Item = usize;
    fn next(&mut self) -> Option<usize> {
        if self.steps_remaining == 0 {
            return None;
        }
        let idx = self.cursor?;
        let entry = *self.thread.stack_table.get(idx)?;
        self.cursor = entry.prefix;
        self.steps_remaining -= 1;
        Some(entry.frame_idx)
    }
}

impl Trace {
    /// Total samples across all threads. Cheap iteration; not cached
    /// because trace mutation is rare and the count is invalidated
    /// by symbolication anyway.
    #[must_use]
    pub fn total_samples(&self) -> u64 {
        self.threads.iter().map(|t| t.samples.len() as u64).sum()
    }

    /// True iff every frame across every thread has a resolved label.
    /// Drives the "should I run the symbolicator?" check before
    /// running an analysis.
    #[must_use]
    pub fn is_fully_symbolicated(&self) -> bool {
        self.threads
            .iter()
            .all(|t| t.resolved.iter().all(Option::is_some))
    }
}
