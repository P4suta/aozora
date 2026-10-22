use std::path::PathBuf;

/// A loaded samply trace.
#[derive(Debug, Clone)]
pub struct Trace {
    /// Loaded objects, indexed by the `lib_idx` stored in each thread's `resource_table`.
    /// Position is the canonical library id.
    pub libs: Vec<Library>,
    /// Profiled threads with their samples and lookup tables.
    pub threads: Vec<Thread>,
    /// Source file containing the trace.
    pub source_path: PathBuf,
}

/// A loaded library or memory mapping.
#[derive(Debug, Clone)]
pub struct Library {
    /// Library name used by [`crate::Symbolicator`] and [`crate::SymbolCache`].
    pub name: String,
    /// Object path at recording time.
    pub path: String,
    /// Separate debug information path, or an empty string.
    pub debug_path: String,
    /// Breakpad debug identifier, or an empty string.
    pub debug_id: String,
    /// Hexadecimal build identifier of the loaded binary.
    pub code_id: String,
}

/// A profiled thread with samples and stack tables.
#[derive(Debug, Clone)]
pub struct Thread {
    /// Thread identifier from the profile producer.
    pub tid: String,
    /// Thread name, or an empty string.
    pub name: String,
    /// Whether this is the process main thread.
    pub is_main: bool,
    /// Processor samples.
    pub samples: Vec<Sample>,

    /// String pool indexed by function names and other tables.
    pub string_array: Vec<String>,
    /// Stack tree traversed by [`Thread::walk_stack`].
    pub stack_table: Vec<StackEntry>,
    /// Frames indexed by stack entries.
    /// Indexed by `StackEntry::frame_idx`.
    pub frame_table: Vec<FrameRow>,
    /// Function names and owning resources.
    /// Indexed by `FrameRow::func_idx`.
    pub func_table: Vec<FuncRow>,
    /// Resource table mapping a function back to its owning library.
    /// Indexed by `FuncRow::resource_idx`.
    pub resource_table: Vec<ResourceRow>,
    /// Resolved function names by frame index, or `None` until resolution.
    /// Same length as `frame_table`.
    pub resolved: Vec<Option<String>>,
}

/// A processor observation with a timestamp, stack, and weight.
#[derive(Debug, Clone, Copy)]
pub struct Sample {
    /// Absolute timestamp in milliseconds.
    /// Zero when the trace carries no timeline.
    pub time_ms: f64,
    /// Leaf index in `stack_table`, or `None` without a captured stack.
    pub stack_idx: Option<usize>,
    /// Number of raw samples this row represents.
    /// Defaults to `1`.
    /// Analyses sum this rather than counting rows.
    pub weight: u64,
}

/// A node in the stack-table tree.
#[derive(Debug, Clone, Copy)]
pub struct StackEntry {
    /// Parent stack index, or `None` at the root.
    /// Following `prefix` walks from leaf to root.
    pub prefix: Option<usize>,
    /// Index in `frame_table` at this depth.
    pub frame_idx: usize,
}

/// An instruction address and its function index.
#[derive(Debug, Clone, Copy)]
pub struct FrameRow {
    pub address: u64,
    pub func_idx: usize,
}

/// One source-language function: its name + owning resource.
#[derive(Debug, Clone, Copy)]
pub struct FuncRow {
    /// Index into [`Thread::string_array`].
    /// Can point at an empty string before symbol resolution.
    pub name_idx: usize,
    pub resource_idx: Option<usize>,
}

/// One resource: maps a function back to its loaded library.
#[derive(Debug, Clone, Copy)]
pub struct ResourceRow {
    pub lib_idx: Option<usize>,
}

impl Thread {
    /// Returns a label for `frame_idx`.
    /// Prefers the symbolicated cache, falls back to the raw `funcTable` string, finally to the hex address.
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

    /// Address recorded for `frame_idx`.
    #[must_use]
    pub fn frame_address(&self, frame_idx: usize) -> u64 {
        self.frame_table[frame_idx].address
    }

    /// Iterate the stack from leaf-most frame to root, yielding frame indices.
    /// Stops after at most `stack_table.len()` steps, including when public tables contain a cycle.
    #[must_use]
    pub fn walk_stack(&self, leaf_stack: usize) -> StackWalker<'_> {
        StackWalker {
            thread: self,
            cursor: Some(leaf_stack),
            steps_remaining: self.stack_table.len(),
        }
    }
}

/// Iterator returned by [`Thread::walk_stack`].
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
    /// Total samples across all threads.
    #[must_use]
    pub fn total_samples(&self) -> u64 {
        self.threads.iter().map(|t| t.samples.len() as u64).sum()
    }

    /// Returns whether every frame across every thread has a resolved label.
    #[must_use]
    pub fn is_fully_symbolicated(&self) -> bool {
        self.threads
            .iter()
            .all(|t| t.resolved.iter().all(Option::is_some))
    }
}
