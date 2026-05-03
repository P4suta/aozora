//! Pure-Rust loader + analyses for samply gecko-format profile traces.
//!
//! `samply record` produces `.json.gz` files in the
//! [Gecko profile](https://github.com/firefox-devtools/profiler/blob/main/docs-developer/gecko-profile-format.md)
//! format. This crate parses them, optionally symbolicates the
//! addresses against the source binary's DWARF info, and exposes a
//! suite of analyses:
//!
//! - [`analysis::hot_leaves`] / [`analysis::hot_inclusive`] — top-N
//!   hot frames, leaf-level (where samples landed) or inclusive
//!   (self + descendants).
//! - [`analysis::library_distribution`] — share of samples spent in
//!   each loaded library (binary vs libc vs vdso vs …).
//! - [`analysis::rollup`] — categorise functions into named buckets
//!   (Phase 1 / Phase 3 / `corpus_load` / etc.) via [`Categorizer`].
//! - [`analysis::matching_stacks`] — print full call stacks where
//!   any frame matches a regex; useful for "why is X being called?".
//! - [`analysis::folded_stacks`] — emit the folded-stack format that
//!   feeds `flamegraph.pl` / inferno.
//! - [`analysis::compare`] — diff two traces (before vs after) and
//!   surface which functions grew, shrank, appeared, or disappeared.
//!
//! ## Workflow
//!
//! ```rust,ignore
//! use std::path::Path;
//! use aozora_trace::{Trace, Symbolicator, analysis::{self, RollupConfig}};
//!
//! // 1. Load the gzipped JSON trace.
//! let mut trace = Trace::load(Path::new("/tmp/aozora-corpus.json.gz"))?;
//!
//! // 2. Symbolicate against the binary that produced it (DWARF lookup).
//! let mut sym = Symbolicator::new();
//! sym.add_binary("throughput_by_class", Path::new("target/release/examples/throughput_by_class"))?;
//! trace.symbolicate(&sym);
//!
//! // 3. Run any analysis. Reports are printable + serialisable.
//! let hot = analysis::hot_leaves(&trace, 25);
//! println!("{}", hot.render_table());
//! ```
//!
//! ## Sidecar cache
//!
//! Symbolication via [`addr2line`] is the slowest step (~seconds per
//! binary on a multi-MB ELF). [`SymbolCache::write`] /
//! [`SymbolCache::load`] persist resolved (lib, address) →
//! function-name mappings to a sidecar JSON next to the trace, so
//! repeated analyses cost milliseconds.

#![forbid(unsafe_code)]
// Pragmatic lint relaxations for this internal dev-tool crate. The
// alternatives (refactoring every `# Errors` doc, stripping every
// `std::collections::HashMap` to a `use`-imported name, hand-checking
// every `as f64` for percentage formatting) inflate code without
// catching real bugs — the strict workspace lints are tuned for the
// production lex pipeline, not for trace post-processing.
#![allow(
    clippy::absolute_paths,
    reason = "use std::collections::HashMap inline; this crate has many short helpers"
)]
#![allow(
    clippy::missing_errors_doc,
    reason = "errors are documented on each error enum (TraceLoadError, SymbolError, CategoryError, CacheError); per-fn # Errors would just restate them"
)]
#![allow(
    clippy::cast_precision_loss,
    reason = "u64 sample counts → f64 percentages; loss is irrelevant at the sub-percent display precision"
)]
#![allow(
    clippy::cast_possible_truncation,
    reason = "u64 → usize for stack/frame index lookups; profile address space is well below 2^32 entries"
)]

mod cache;
mod categories;
mod load;
mod render;
mod symbol;
mod trace;

pub mod analysis;

pub use cache::{LibIdent, SymbolCache};
pub use categories::{Categorizer, RollupConfig};
pub use load::TraceLoadError;
pub use render::TableRenderable;
pub use symbol::{SymbolError, Symbolicator};
pub use trace::{Library, Sample, StackEntry, Thread, Trace};
