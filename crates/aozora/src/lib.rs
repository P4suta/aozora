//! `aozora` — the public meta crate.
//!
//! Single front door for parsing Aozora Bunko notation. Downstream
//! consumers should depend on this crate alone; everything they need
//! is re-exported through this surface or accessed via [`Document`]
//! and [`AozoraTree`].
//!
//! ```no_run
//! use aozora::Document;
//!
//! let source = std::fs::read_to_string("crime_and_punishment.txt").unwrap();
//! let doc = Document::new(source);
//! let tree = doc.parse();
//! let html = tree.to_html();
//! println!("{html}");
//! ```
//!
//! ## Move 3 façade phase
//!
//! Today [`Document`] / [`AozoraTree`] are thin façades over the
//! legacy [`aozora_parser::ParseResult`]. Once Move 2's fused engine
//! lands, [`Document`] will own a [`bumpalo::Bump`] arena and
//! [`AozoraTree`] will borrow from it directly per the architecture
//! described in ADR-0009 / ADR-0010. The public API shape stays
//! source-compatible across the migration.

#![forbid(unsafe_code)]

pub use aozora_lex::{
    BLOCK_CLOSE_SENTINEL, BLOCK_LEAF_SENTINEL, BLOCK_OPEN_SENTINEL, INLINE_SENTINEL, LexOutput,
    PlaceholderRegistry, lex,
};
pub use aozora_parser::{ParseArtifacts, ParseResult, parse};
// During Plan B's incremental migration, `aozora::html` continues to
// expose the legacy owned-AST renderer (drives `render_from_artifacts`
// from FFI/WASM/Py drivers). The borrowed-AST native renderer is
// reachable via `aozora_render::html` directly. Plan B.4 switches this
// re-export over.
pub use aozora_render::legacy::html;
pub use aozora_render::legacy::{serialize, serialize_from_artifacts};
pub use aozora_spec::{Diagnostic, PairKind, Span, TriggerKind};

mod document;

pub use document::{AozoraTree, Document};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_parse_returns_a_tree() {
        let doc = Document::new("hello, world");
        let tree = doc.parse();
        // Plain text round-trips intact.
        assert_eq!(tree.serialize(), "hello, world");
    }

    #[test]
    fn document_parse_handles_ruby() {
        let doc = Document::new("｜青梅《おうめ》");
        let tree = doc.parse();
        // Round-trip preserves the canonical form.
        assert_eq!(tree.serialize(), "｜青梅《おうめ》");
    }

    #[test]
    fn document_to_html_renders_plain_text() {
        let doc = Document::new("hello");
        let tree = doc.parse();
        let html = tree.to_html();
        assert!(html.contains("hello"), "html: {html}");
    }
}
