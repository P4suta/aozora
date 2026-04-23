//! HTML renderer for Aozora AST nodes.
//!
//! Produces semantic HTML5. The paired stylesheet shipped in `afm-vertical.css` /
//! `afm-horizontal.css` applies the visual styling; this module only emits structure
//! so the same output works in printed, epub, and browser contexts.

// M0 Spike: renderer lands alongside AST population once the fork wiring exists.
