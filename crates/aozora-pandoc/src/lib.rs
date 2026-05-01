//! Aozora → Pandoc AST projection.
//!
//! `aozora-pandoc` lifts an [`aozora::AozoraTree`] into a
//! [`pandoc_ast::Pandoc`] document so any of Pandoc's 50+ output
//! formats (HTML, EPUB, LaTeX/PDF, DOCX, …) can render Aozora Bunko
//! notation without each format growing its own Aozora codepath.
//!
//! ## Architecture
//!
//! Aozora has rich semantic markup that no single Pandoc native
//! construct captures (ruby, bouten, tate-chu-yoko, gaiji, …).
//! The crate maps each [`aozora::AozoraNode`] variant to a Pandoc
//! [`pandoc_ast::Inline::Span`] / [`pandoc_ast::Block::Div`] with
//! a stable CSS class (e.g. `aozora-ruby`, `aozora-bouten`) plus
//! attribute key/value pairs carrying the structured data
//! (e.g. ruby base + reading, bouten kind + position).
//!
//! That translation is **format-agnostic** by construction: every
//! Pandoc writer renders `Span` / `Div` as a stylable container
//! (`<span class="aozora-ruby">` for HTML, `\\textit{…}` fallback
//! for LaTeX, etc.). Downstream consumers wanting format-native
//! markup (`<ruby><rt>…</rt></ruby>` instead of `<span>`) hook in
//! a Pandoc filter that pattern-matches on these CSS classes.
//!
//! ## Smarter-than-naive choice
//!
//! Naive routes considered and rejected:
//!
//! 1. **`RawInline("html", "<ruby>…</ruby>")`** — fast for HTML but
//!    every other output format strips the raw HTML, defeating the
//!    point of going through Pandoc.
//! 2. **`Plain` blocks of bare text** — loses every semantic
//!    distinction (ruby reading collapses into base text).
//! 3. **One Pandoc filter per output format** — multiplies surface
//!    area; the format-agnostic span representation lets a single
//!    filter (or none, with CSS) handle every format.
//!
//! The Span-with-class projection is the same pattern Pandoc itself
//! uses for `[content]{.smallcaps}` and what HTML5/EPUB authors
//! use to attach semantic meaning that CSS / XSL can transform
//! later.
//!
//! ## Usage
//!
//! ```no_run
//! use aozora::Document;
//! use aozora_pandoc::to_pandoc;
//!
//! let doc = Document::new("｜青梅《おうめ》");
//! let tree = doc.parse();
//! let pandoc = to_pandoc(&tree);
//! // Serialize to Pandoc JSON for `pandoc -f json -t html`:
//! let json = serde_json::to_string(&pandoc).expect("serialise pandoc ast");
//! println!("{json}");
//! ```

#![forbid(unsafe_code)]

mod project;

pub use project::to_pandoc;

/// CSS-class prefix every Aozora-flavoured Pandoc Span/Div carries.
/// Stable: a downstream Pandoc filter that wants to specialise the
/// rendering matches on this prefix.
pub const AOZORA_CLASS_PREFIX: &str = "aozora-";
