//! Aozora → Pandoc AST projection.
//!
//! Lifts a parsed aozora document into a [`pandoc_ast::Pandoc`] document so any
//! of Pandoc's 50+ output formats (HTML, EPUB, LaTeX/PDF, DOCX, …) can render
//! Aozora Bunko notation without each format growing its own Aozora codepath.
//!
//! ## Architecture
//!
//! Standard emphases map to their native Pandoc construct — 太字 →
//! [`pandoc_ast::Inline::Strong`], 斜体 → [`pandoc_ast::Inline::Emph`],
//! 上付き / 下付き小文字 → [`pandoc_ast::Inline::Superscript`] /
//! [`pandoc_ast::Inline::Subscript`]. The rest of Aozora's semantic markup has
//! no single native construct (ruby, bouten, tate-chu-yoko, gaiji, font size,
//! 囲み, …); the projection maps each such [`Node`](crate::Node) variant to a
//! Pandoc [`pandoc_ast::Inline::Span`] / [`pandoc_ast::Block::Div`] with a
//! stable CSS class (e.g. `aozora-ruby`, `aozora-bouten`) plus attribute
//! key/value pairs carrying the structured data (e.g. ruby base + reading,
//! bouten kind + position).
//!
//! That translation is **format-agnostic** by construction: every Pandoc writer
//! renders `Span` / `Div` as a stylable container (`<span class="aozora-ruby">`
//! for HTML, `\\textit{…}` fallback for LaTeX, etc.). Downstream consumers
//! wanting format-native markup (`<ruby><rt>…</rt></ruby>` instead of `<span>`)
//! hook in a Pandoc filter that pattern-matches on these CSS classes.
//!
//! The Span-with-class projection is the same pattern Pandoc itself uses for
//! `[content]{.smallcaps}` and what HTML5/EPUB authors use to attach semantic
//! meaning that CSS / XSL can transform later.
//!
//! ## Usage
//!
//! ```
//! use aozora::Document;
//!
//! let doc = Document::new("｜青梅《おうめ》");
//! let snapshot = doc.snapshot();
//! let pandoc = aozora::pandoc::to_pandoc(&snapshot);
//! // The ruby base lands in the Pandoc AST.
//! assert!(format!("{:?}", pandoc.blocks).contains("青梅"));
//! ```

mod project;

pub use project::to_pandoc;

/// CSS-class prefix every Aozora-flavoured Pandoc Span/Div carries.
/// Stable: a downstream Pandoc filter that wants to specialise the
/// rendering matches on this prefix.
pub const AOZORA_CLASS_PREFIX: &str = "aozora-";
