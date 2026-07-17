//! Lossless concrete syntax tree (CST).
//!
//! Builds a [rowan][rowan]-backed [`SyntaxNode`](crate::cst::SyntaxNode) tree as a **pure projection**
//! over a parsed [`Tree`](crate::Tree) — no changes to the lex pipeline are
//! required. The decoupled architecture means the CST stays reproducible from
//! source bytes alone, and adding/removing CST consumers does not perturb the
//! AST's perf-critical path.
//!
//! ## Lossless invariant
//!
//! Concatenating every leaf token's text yields exactly the sanitized source:
//!
//! ```
//! use aozora::Document;
//! use rowan::{NodeOrToken, WalkEvent};
//!
//! let doc = Document::new("｜青梅《おうめ》");
//! let tree = doc.parse();
//! let cst = aozora::cst::from_tree(&tree);
//! let reconstructed: String = cst
//!     .preorder_with_tokens()
//!     .filter_map(|step| match step {
//!         WalkEvent::Enter(NodeOrToken::Token(t)) => Some(t.text().to_owned()),
//!         _ => None,
//!     })
//!     .collect();
//! assert_eq!(reconstructed, tree.sanitized());
//! ```
//!
//! That property is the reason rowan exists; it is what enables
//! comment-preserving formatters, source-faithful refactoring, and editor-grade
//! syntax highlighting that survive minor parser changes.
//!
//! ## Granularity
//!
//! The classifier emits per-construct spans. The CST projection treats each
//! span as a `Construct` node containing one `ConstructText` token whose bytes
//! equal the source slice; bytes between spans become standalone `Plain` text
//! tokens. Container open / close events nest the intervening blocks under a
//! `Container` node so editor outlines can collapse / expand a paired indent /
//! keigakomi region.
//!
//! Finer per-token granularity (individual punctuation, kana runs, …) is a
//! deliberate non-goal: the projection stays construct-level because the
//! lossless property already holds at this granularity. A future consumer that
//! needs sub-construct tokens can refine it without disturbing this contract.
//!
//! [rowan]: https://docs.rs/rowan

mod kind;
mod project;

use crate::Tree;
use crate::pipeline::lex;
pub use kind::{AozoraLanguage, SyntaxKind, SyntaxNode, SyntaxToken};
pub use project::build_cst;

/// Build the CST directly from a parsed [`Tree`].
///
/// Sanitizes the tree's source and projects its classified source-node table
/// into a rowan [`SyntaxNode`]. The leaf-text concatenation equals the tree's
/// [`sanitized`](crate::Tree::sanitized) source (the lossless invariant; note
/// this is the *sanitized* contract, not the original source — the two are
/// byte-identical on inputs that triggered no sanitize rewrite).
#[must_use]
pub fn from_tree(tree: &Tree<'_>) -> SyntaxNode {
    let out = lex(tree.source());
    build_cst(&out.sanitized, &out.source_nodes)
}
