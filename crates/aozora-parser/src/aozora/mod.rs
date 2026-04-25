//! Aozora render helpers.
//!
//! Renderer-only: every recogniser lives in `aozora-lexer` (Phase 3
//! classification). The block-level walker in [`crate::html`] consumes
//! the lexer's normalized text + registry directly and dispatches each
//! sentinel to [`html::render`] (this module) for the per-node markup.

pub mod bouten;
pub mod classes;
pub mod html;

pub use classes::AFM_CLASSES;
