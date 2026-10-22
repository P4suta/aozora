//! Projects Aozora documents to Pandoc.
//!
//! Notation without a native Pandoc construct uses spans or divs with Aozora classes.
//!
//! ```
//! use aozora::parse;
//!
//! let doc = parse("｜青梅《おうめ》").expect("source fits parser limits");
//! let snapshot = doc.snapshot();
//! let pandoc = aozora::pandoc::to_pandoc(&snapshot);
//! assert!(format!("{:?}", pandoc.blocks).contains("青梅"));
//! ```

mod project;

pub use project::to_pandoc;

/// CSS-class prefix for Aozora Pandoc spans and divs.
pub const AOZORA_CLASS_PREFIX: &str = "aozora-";
