//! Aozora render helpers.
//!
//! Renderer-only: every recogniser lives in `afm-lexer` (Phase 3
//! classification) and every AST-surgery pass lives in
//! `afm-parser::post_process`. ADR-0008 keeps the render-side `fn`
//! pointer as the only surviving comrak/afm seam.

pub mod bouten;
pub mod html;
