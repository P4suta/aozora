//! Zero-copy, arena-allocated AST.
//!
//! This module is the AST that the `aozora-lex` pipeline produces and
//! that downstream consumers (`aozora-render`, `aozora`, the FFI /
//! WASM / Python drivers) walk.
//!
//! # Lifetime model
//!
//! Every type carries a single lifetime parameter `'src`, the
//! lifetime of the source text being parsed *and* of the arena
//! allocator that owns the tree's storage. By convention the
//! enclosing `Document<'src>` owns both, so `'src` is the borrow of
//! that document.
//!
//! All AST types are `Copy` because they only contain `Copy` data:
//! `&'src` references, primitives, and `Copy` enums. This means a
//! parsed [`AozoraNode`] can be passed by value without ceremony and
//! the visitor pattern in `aozora-render` does not need
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
//! Every type here borrows its payload from the source / arena
//! rather than owning a heap copy. The "observable equivalence"
//! purity contract permits arena mutation behind the scenes while
//! keeping the public surface deterministic.

mod arena;
mod intern;
mod non_empty;
mod registry;
mod types;

pub use arena::Arena;
pub use intern::{InternStats, Interner};
pub use non_empty::NonEmpty;
pub use registry::{ContainerPair, NodeRef, Registry};
pub use types::{
    Annotation, AozoraHeading, AozoraNode, Bouten, Content, DoubleRuby, Gaiji, HeadingHint,
    Kaeriten, Ruby, Sashie, Segment, TateChuYoko, Warichu,
};
