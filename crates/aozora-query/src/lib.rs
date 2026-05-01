//! Tree-sitter-flavoured pattern queries over the aozora CST.
//!
//! `aozora-query` ships a tiny DSL that selects nodes from a
//! [`aozora_cst::SyntaxNode`] tree, modelled on the
//! [tree-sitter query language][ts-query]. Editor surfaces (LSP
//! `textDocument/documentHighlight`, "find all ruby annotations",
//! refactoring filters) compose against the DSL instead of
//! re-implementing tree walks.
//!
//! ## DSL grammar
//!
//! ```text
//! query   := pattern ('\n' pattern)* '\n'?
//! pattern := '(' kind capture? ')'
//!          | '(' '_'  capture? ')'
//! kind    := SyntaxKind ident      // e.g. `Construct`, `Container`
//! capture := '@' ident
//! ident   := [A-Za-z_][A-Za-z0-9_-]*
//! ```
//!
//! - `(Construct)` matches every `Construct` node.
//! - `(Construct @ruby)` captures each `Construct` under the name
//!   `ruby` so the iterator yields one [`Capture`] per match.
//! - `(_)` matches any node kind; combine with `@name` for a
//!   "tour every node" walker.
//! - Multiple patterns separated by newlines run as an OR — every
//!   matching node yields one capture per pattern that hits.
//!
//! ## Execution model
//!
//! Patterns compile once into a [`Query`] (a vector of pattern
//! atoms), then `Query::captures` walks the CST in preorder
//! invoking the cheap `kind` match at every node. The DSL is
//! intentionally tiny — predicates (`#eq?`, `#match?`), field
//! accessors, and quantifiers wait until a concrete consumer asks
//! for them. The API shape (compile-once, iterate captures) is
//! forward-compatible with that growth.
//!
//! [ts-query]: https://tree-sitter.github.io/tree-sitter/using-parsers/queries

#![forbid(unsafe_code)]

mod compile;
mod matcher;

pub use compile::{Query, QueryError, compile};
pub use matcher::Capture;
