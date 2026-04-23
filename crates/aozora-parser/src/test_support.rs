//! Minimal test utilities for afm-parser.
//!
//! Deliberately tiny: everything richer (pretty diffs, snapshots, property tests,
//! error reporting) is outsourced to the industry-standard Rust testing crates —
//! `pretty_assertions`, `insta`, `proptest`, `miette` — rather than re-implemented
//! here. This module exists only for traversal glue that the stdlib alone can't
//! express.

use comrak::Arena;
use comrak::nodes::AstNode;

use crate::{Options, parse};

/// Parse `input` with afm defaults and collect every [`afm_syntax::AozoraNode`] in
/// document order. Drives behavioural tests that care about "which recognisers
/// fired" rather than the shape of the arena tree.
#[must_use]
pub fn collect_aozora(input: &str) -> Vec<afm_syntax::AozoraNode> {
    let arena = Arena::new();
    let opts = Options::afm_default();
    let root = parse(&arena, input, &opts);
    let mut out = Vec::new();
    walk(root, &mut out);
    out
}

fn walk<'a>(node: &'a AstNode<'a>, out: &mut Vec<afm_syntax::AozoraNode>) {
    collect_aozora_recursive(node, out);
}

/// Recursive traversal helper usable by tests that already hold an [`AstNode`]
/// (e.g. when testing parse modes that bypass the default arena).
pub fn collect_aozora_recursive<'a>(node: &'a AstNode<'a>, out: &mut Vec<afm_syntax::AozoraNode>) {
    if let comrak::nodes::NodeValue::Aozora(ref boxed) = node.data.borrow().value {
        out.push((**boxed).clone());
    }
    for child in node.children() {
        collect_aozora_recursive(child, out);
    }
}
