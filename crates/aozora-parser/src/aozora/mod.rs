//! Aozora render helpers.
//!
//! Post-ADR-0008 cutover this module only holds the HTML renderer and a
//! small slug helper it depends on. All of the recognisers that formerly
//! lived under `aozora::{annotation, block, inline, layout, ruby, tcy}`
//! are now implemented in `afm-lexer` (Phase 3 classification) and
//! `afm-parser::post_process` (AST surgery), so the adapter no longer
//! dispatches inline or block parse hooks.

pub mod bouten;
pub mod html;
