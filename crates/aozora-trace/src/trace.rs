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
    /// Loaded objects, indexed by the `lib_idx` stored in each
    /// thread's `resource_table`. Position is the canonical library id.
    pub libs: Vec<Library>,
    /// Profiled threads, each carrying its own sample stream and the
    /// stack/frame/func tables those samples index into.
    pub threads: Vec<Thread>,
    /// Source file the trace was loaded from. Drives sidecar-cache
    /// path resolution and shows up in diagnostic prints.
    pub source_path: PathBuf,
}

/// A loaded library (ELF file, vdso, or anonymous mapping).
#[derive(Debug, Clone)]
pub struct Library {
    /// Short library name (e.g. `libc.so.6`, the binary's file name).
    /// Used as the cross-reference key against [`crate::Symbolicator`]
    /// registrations and [`crate::SymbolCache`] entries.
    pub name: String,
    /// Filesystem path the object was loaded from at record time.
    /// [`crate::Symbolicator::add_libs_from`] re-opens it for DWARF.
    pub path: String,
    /// Path to the separate debuginfo file, when the recorder split
    /// it out. Often equal to `path` or empty.
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
    /// Thread id as recorded by the profile producer.
    pub tid: String,
    /// Thread name (e.g. `main`, a tokio worker name), or empty.
    pub name: String,
    /// Whether this is the process main thread.
    pub is_main: bool,
    /// The CPU sample stream. Each [`Sample`] points (via `stack_idx`)
    /// into `stack_table`; analyses iterate this to count time.
    pub samples: Vec<Sample>,

    // Internal tables — exposed `pub` because the analysis modules
    // consult them directly. Mutating from outside the crate is
    // unsupported (no invariant checking).
    /// String interning pool. `FuncRow::name_idx` and other tables
    /// index into this rather than carrying owned strings.
    pub string_array: Vec<String>,
    /// Stack-table tree: each [`StackEntry`] is `(prefix, frame)`,
    /// so a stack id chains up its `prefix` to the root. Walked by
    /// [`Thread::walk_stack`].
    pub stack_table: Vec<StackEntry>,
    /// Frame table: per-frame `(address, func_idx)`. Indexed by
    /// `StackEntry::frame_idx`.
    pub frame_table: Vec<FrameRow>,
    /// Function table: per-function `(name_idx, resource_idx)`.
    /// Indexed by `FrameRow::func_idx`.
    pub func_table: Vec<FuncRow>,
    /// Resource table mapping a function back to its owning library.
    /// Indexed by `FuncRow::resource_idx`.
    pub resource_table: Vec<ResourceRow>,
    /// Resolved function names per frame index — `None` until
    /// [`crate::Symbolicator`] fills them in. Same length as
    /// `frame_table`.
    pub resolved: Vec<Option<String>>,
}

/// A single CPU sample.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    /// Absolute timestamp in milliseconds. Reconstructed from the
    /// trace's `time` or `timeDeltas` column; `0.0` when the trace
    /// carried no timeline (timestamps aren't load-bearing for the
    /// sample-counting analyses).
    pub time_ms: f64,
    /// Leaf stack id (index into `stack_table`), or `None` for an
    /// empty/idle sample with no captured stack.
    pub stack_idx: Option<usize>,
    /// Sample weight, i.e. how many raw samples this row represents
    /// (samply may coalesce identical consecutive samples). Defaults
    /// to `1`. Analyses sum this rather than counting rows.
    pub weight: u64,
}

/// A node in the stack-table tree. `prefix.prefix.prefix...` chain
/// terminates at `None` (the root).
#[derive(Debug, Clone, Copy)]
pub struct StackEntry {
    /// Parent stack id (the caller frame's stack node), or `None` at
    /// the root. Following `prefix` repeatedly walks leaf → root.
    pub prefix: Option<usize>,
    /// The frame at this stack depth (index into `frame_table`).
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
    /// frame indices. The loader rejects prefix cycles; the iterator
    /// remains bounded because callers can mutate the public tables.
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
