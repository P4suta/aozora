//! Loads samply Gecko profiles through [`Trace`], resolves symbols through [`Symbolicator`], and produces reports through [`analysis`].
//!
//! ```rust,ignore
//! use std::path::Path;
//! use aozora_trace::{Trace, Symbolicator, SymbolCache, TableRenderable, analysis};
//!
//! // 1. Load the gzipped JSON trace.
//! let mut trace = Trace::load(Path::new("/tmp/aozora-corpus.json.gz"))?;
//!
//! // 2. Symbolicate against the binary that produced it (DWARF lookup).
//! let mut sym = Symbolicator::new();
//! sym.add_binary("throughput_by_class", Path::new("target/release/examples/throughput_by_class"))?;
//! let mut cache = SymbolCache::default();
//! sym.resolve_into(&mut trace, &mut cache);
//!
//! // 3. Run any analysis. Reports are printable + serialisable.
//! let hot = analysis::hot_leaves(&trace, 25);
//! println!("{}", hot.render_table());
//! ```

#![forbid(unsafe_code)]

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
