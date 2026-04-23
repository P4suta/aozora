//! Aozora Bunko extensions layered on top of the vendored comrak fork.
//!
//! This module is the single entry point that `upstream/comrak/src/parser/inline.rs`
//! and `.../block.rs` call into through the hook-line additions. Keeping everything
//! here means the upstream diff is bounded to one `use` + one dispatch call per file.

pub mod annotation;
pub mod block;
pub mod html;
pub mod inline;
pub mod ruby;
