//! Lossless concrete syntax tree (CST) for aozora.
//!
//! `aozora-cst` builds a [rowan][rowan]-backed `SyntaxNode` tree as
//! a **pure projection** over the public surface of
//! [`aozora::AozoraTree`] — no changes to the lex pipeline are
//! required. The decoupled architecture means the CST stays
//! reproducible from source bytes alone, and adding/removing CST
//! consumers does not perturb the AST's perf-critical path.
//!
//! ## Lossless invariant
//!
//! Concatenating every leaf token's text yields exactly the
//! original source bytes:
//!
//! ```rust,ignore
//! let cst = aozora_cst::build_cst(&tree);
//! let reconstructed: String = cst
//!     .preorder_with_tokens()
//!     .filter_map(|step| match step {
//!         rowan::WalkEvent::Enter(rowan::NodeOrToken::Token(t)) => Some(t.text().to_owned()),
//!         _ => None,
//!     })
//!     .collect();
//! assert_eq!(reconstructed, tree.source());
//! ```
//!
//! That property is the reason rowan exists; it is what enables
//! comment-preserving formatters, source-faithful refactoring, and
//! editor-grade syntax highlighting that survive minor parser
//! changes.
//!
//! ## Granularity
//!
//! aozora's classifier emits per-construct spans
//! ([`aozora::SourceNode`]). The CST projection treats each span
//! as a `Construct` node containing one `Text` token whose bytes
//! equal the source slice; bytes between spans become standalone
//! `Plain` text tokens. Container open / close events nest the
//! intervening blocks under a `Container` node so editor outlines
//! can collapse / expand a paired indent / keigakomi region.
//!
//! Finer per-token granularity (individual punctuation, kana
//! runs, …) is a Phase 0.5+ extension once we have a concrete
//! consumer asking for it. The lossless property holds at any
//! granularity.
//!
//! [rowan]: https://docs.rs/rowan

#![forbid(unsafe_code)]

mod build;
mod kind;

pub use build::build_cst;
pub use kind::{AozoraLanguage, SyntaxKind, SyntaxNode, SyntaxToken};
