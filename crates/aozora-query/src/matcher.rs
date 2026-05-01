//! Query execution: walk the CST in preorder, emit a [`Capture`]
//! for each pattern hit. Walks both nodes and tokens so queries
//! can match tokenkinds (`ContainerOpen`, `ContainerClose`,
//! `Plain`, `ConstructText`) as well as branch kinds.

use aozora_cst::{SyntaxKind, SyntaxNode};
use rowan::NodeOrToken;

use crate::compile::{Pattern, PatternKind, Query};

/// One pattern match — node or token.
///
/// `node` is `NodeOrToken<SyntaxNode, SyntaxToken>` so a query can
/// capture either a branch (e.g. `Container`) or a token (e.g.
/// `ContainerOpen`). Consumers extract `node.text()` for the
/// matched source slice or `node.text_range()` for the byte range.
pub(crate) type CaptureTarget = NodeOrToken<SyntaxNode, aozora_cst::SyntaxToken>;

/// One pattern match. Carries the matched node-or-token together
/// with the capture name from the pattern (or empty if the pattern
/// omitted `@name`).
#[derive(Debug, Clone)]
pub struct Capture {
    /// Capture name (`@name` in the DSL); empty string when the
    /// pattern had no `@`-clause.
    pub name: String,
    /// The CST node or token that matched.
    pub node: CaptureTarget,
}

impl Query {
    /// Iterate every match in preorder.
    ///
    /// Each pattern in the [`Query`] is tested against every node
    /// and token; a node that satisfies multiple patterns yields
    /// one [`Capture`] per pattern. Yields a `Vec` (not a streaming
    /// iterator) so consumers can sort / filter / count without
    /// fighting the borrow checker — captures hold rowan handles,
    /// which are cheap to clone.
    #[must_use]
    pub fn captures(&self, root: &SyntaxNode) -> Vec<Capture> {
        let mut out = Vec::new();
        for step in root.preorder_with_tokens() {
            let rowan::WalkEvent::Enter(target) = step else {
                continue;
            };
            let kind: SyntaxKind = match &target {
                NodeOrToken::Node(n) => n.kind(),
                NodeOrToken::Token(t) => t.kind(),
            };
            for pattern in &self.patterns {
                if pattern_matches(pattern, kind) {
                    out.push(Capture {
                        name: pattern.capture.clone().unwrap_or_default(),
                        node: target.clone(),
                    });
                }
            }
        }
        out
    }
}

fn pattern_matches(pattern: &Pattern, kind: SyntaxKind) -> bool {
    match pattern.kind {
        PatternKind::Any => true,
        PatternKind::Kind(want) => want == kind,
    }
}
