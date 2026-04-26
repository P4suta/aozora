//! Zero-copy, arena-allocated AST.
//!
//! This module provides the AST shape that the new `aozora-lex`
//! pipeline (Move 2 of the 0.2.0 plan) will produce. It coexists
//! with the legacy owned AST in the parent module so the workspace
//! stays green during the multi-Move migration; once `aozora-lex`
//! ships and downstream consumers (`aozora-render`, `aozora-parallel`,
//! `aozora`) are wired against this module, the owned AST will be
//! deprecated and eventually removed.
//!
//! # Lifetime model
//!
//! Every type carries a single lifetime parameter `'src`, the
//! lifetime of the source text being parsed *and* of the arena
//! allocator that owns the tree's storage. By convention the
//! enclosing `Document<'src>` (delivered in Move 3) owns both, so
//! `'src` is the borrow of that document.
//!
//! All AST types are `Copy` because they only contain `Copy` data:
//! `&'src` references, primitives, and `Copy` enums. This means a
//! parsed [`AozoraNode`] can be passed by value without ceremony and
//! the visitor pattern in `aozora-render` (Move 3) does not need
//! `&mut` self for traversal.
//!
//! # Memory ownership
//!
//! Construction allocates into an [`Arena`] (a thin wrapper over
//! `bumpalo::Bump`). Every `&'src str` inside the tree points either
//! to the arena (rewritten / synthesised text) or to the source
//! string (zero-copy borrow of original bytes). When the arena drops,
//! the entire tree drops as a single deallocation; per-node `Drop`
//! never runs.
//!
//! # Why "borrowed"?
//!
//! The module name reflects the contrast with the legacy `Box<str>` /
//! `Box<AozoraNode>`-based owned AST: every type here borrows its
//! payload rather than owning a heap copy. ADR-0010 codifies this
//! shift and the "observable equivalence" purity contract that
//! permits arena mutation behind the scenes.

mod arena;
mod intern;
mod registry;
mod types;

pub use arena::Arena;
pub use intern::{InternStats, Interner};
pub use registry::{BlockRegistry, ContainerRegistry, InlineRegistry, Registry};
pub use types::{
    Annotation, AozoraHeading, AozoraNode, Bouten, Content, DoubleRuby, Gaiji, HeadingHint,
    Kaeriten, Ruby, Sashie, Segment, TateChuYoko, Warichu,
};
