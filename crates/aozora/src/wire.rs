//! Driver-shared wire format for serialising `aozora` parser output.
//!
//! Three driver crates (`aozora-ffi`, `aozora-wasm`, `aozora-py`) all
//! need to project the borrowed-AST output to a stable byte stream.
//! This module is the **single authority** for that projection — each
//! driver calls into here and is guaranteed bit-identical output
//! across language boundaries.
//!
//! # Schema envelope
//!
//! Every wire JSON has the shape
//!
//! ```json
//! { "schema_version": 1, "data": [ /* …entries… */ ] }
//! ```
//!
//! [`SCHEMA_VERSION`] is bumped on any breaking change to the
//! serialised shape (variant additions, field renames, envelope
//! changes). Clients that read the wire format SHOULD branch on the
//! version to decide their handling — schema 1 makes no guarantees of
//! forward-compatibility with later schemas.
//!
//! # Stability vs. `non_exhaustive`
//!
//! Both [`crate::Diagnostic`] and [`crate::AozoraNode`] are
//! `#[non_exhaustive]` upstream so the library can add variants in
//! minor releases. The wire format protects callers by:
//!
//! - falling back to `kind: "unknown"` for unrecognised variants, and
//! - bumping [`SCHEMA_VERSION`] when new variants land in the wire
//!   (so a client that branches on the version can react before
//!   `"unknown"` shows up in production traffic).

use serde::Serialize;

use crate::{AozoraNode, AozoraTree, Diagnostic, NodeRef, PairKind, Span};

/// Wire-format schema version. Bumped on any breaking change to the
/// serialised shape (variant additions, field renames, envelope
/// changes).
pub const SCHEMA_VERSION: u32 = 1;

/// Project a slice of [`Diagnostic`] into a `{ schema_version, data }`
/// JSON envelope. Every entry has the shape
/// `{ kind, span: { start, end }, codepoint? }`.
///
/// Empty input → `{"schema_version":1,"data":[]}`.
#[must_use]
pub fn serialize_diagnostics(diagnostics: &[Diagnostic]) -> String {
    let entries: Vec<DiagnosticWire> = diagnostics.iter().map(DiagnosticWire::from).collect();
    serialize_envelope(&entries)
}

/// Project an [`AozoraTree`]'s source-keyed node side-table into a
/// `{ schema_version, data }` JSON envelope.
///
/// Every entry has the shape `{ kind, span: { start, end } }`,
/// source-coordinate, sorted by `span.start`. Empty parse →
/// `{"schema_version":1,"data":[]}`.
#[must_use]
pub fn serialize_nodes(tree: &AozoraTree<'_>) -> String {
    let entries: Vec<NodeWire> = tree
        .source_nodes()
        .iter()
        .map(|sn| NodeWire {
            kind: node_ref_kind_str(sn.node),
            span: sn.source_span.into(),
        })
        .collect();
    serialize_envelope(&entries)
}

/// Project an [`AozoraTree`]'s pair table into a
/// `{ schema_version, data }` JSON envelope. Every entry has the shape
/// `{ kind, open: { start, end }, close: { start, end } }`.
///
/// One entry per matched open/close pair; unmatched closes and
/// unclosed opens are excluded (they have no partner span and would
/// only confuse editor surfaces). Useful for LSP requests like
/// `textDocument/linkedEditingRange` and
/// `textDocument/documentHighlight`.
///
/// Empty parse → `{"schema_version":1,"data":[]}`.
#[must_use]
pub fn serialize_pairs(tree: &AozoraTree<'_>) -> String {
    let entries: Vec<PairWire> = tree
        .pairs()
        .iter()
        .map(|link| PairWire {
            kind: pair_kind_str(link.kind),
            open: link.open.into(),
            close: link.close.into(),
        })
        .collect();
    serialize_envelope(&entries)
}

/// Stable camelCase tag for an [`AozoraNode`] variant. Centralised
/// here so every driver agrees on the wire spelling.
///
/// Forward-compatible with `#[non_exhaustive]` upstream — unknown
/// variants fall through to `"unknown"`. [`SCHEMA_VERSION`] is bumped
/// whenever a new variant lands in the wire.
#[must_use]
pub fn node_kind_str(node: &AozoraNode<'_>) -> &'static str {
    match node {
        AozoraNode::Ruby(_) => "ruby",
        AozoraNode::Bouten(_) => "bouten",
        AozoraNode::TateChuYoko(_) => "tateChuYoko",
        AozoraNode::Gaiji(_) => "gaiji",
        AozoraNode::Indent(_) => "indent",
        AozoraNode::AlignEnd(_) => "alignEnd",
        AozoraNode::Warichu(_) => "warichu",
        AozoraNode::Keigakomi(_) => "keigakomi",
        AozoraNode::PageBreak => "pageBreak",
        AozoraNode::SectionBreak(_) => "sectionBreak",
        AozoraNode::AozoraHeading(_) => "heading",
        AozoraNode::HeadingHint(_) => "headingHint",
        AozoraNode::Sashie(_) => "sashie",
        AozoraNode::Kaeriten(_) => "kaeriten",
        AozoraNode::Annotation(_) => "annotation",
        AozoraNode::DoubleRuby(_) => "doubleRuby",
        AozoraNode::Container(_) => "container",
        _ => "unknown",
    }
}

/// Stable camelCase tag for a [`NodeRef`].
///
/// Inline / block-leaf hits project to the underlying [`AozoraNode`]
/// variant via [`node_kind_str`]; container open / close hits are
/// flattened into `"containerOpen"` / `"containerClose"` because the
/// wire format places container kind detail in the inline span, not
/// in the open/close marker.
#[must_use]
pub fn node_ref_kind_str(noderef: NodeRef<'_>) -> &'static str {
    match noderef {
        NodeRef::Inline(node) | NodeRef::BlockLeaf(node) => node_kind_str(&node),
        NodeRef::BlockOpen(_) => "containerOpen",
        NodeRef::BlockClose(_) => "containerClose",
        _ => "unknown",
    }
}

/// Stable camelCase tag for a [`PairKind`]. `non_exhaustive` upstream;
/// unknown pair kinds fall through to `"unknown"`.
#[must_use]
pub fn pair_kind_str(kind: PairKind) -> &'static str {
    match kind {
        PairKind::Bracket => "bracket",
        PairKind::Ruby => "ruby",
        PairKind::DoubleRuby => "doubleRuby",
        PairKind::Tortoise => "tortoise",
        PairKind::Quote => "quote",
        _ => "unknown",
    }
}

// ────────────────────────────────────────────────────────────────────
// Internal: envelope + wire structs
// ────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct Envelope<'a, T> {
    schema_version: u32,
    data: &'a [T],
}

fn serialize_envelope<T: Serialize>(data: &[T]) -> String {
    let env = Envelope {
        schema_version: SCHEMA_VERSION,
        data,
    };
    serde_json::to_string(&env)
        .unwrap_or_else(|_| format!(r#"{{"schema_version":{SCHEMA_VERSION},"data":[]}}"#))
}

#[derive(Serialize)]
struct SpanWire {
    start: u32,
    end: u32,
}

impl From<Span> for SpanWire {
    fn from(s: Span) -> Self {
        Self {
            start: s.start,
            end: s.end,
        }
    }
}

#[derive(Serialize)]
struct DiagnosticWire {
    kind: &'static str,
    span: SpanWire,
    #[serde(skip_serializing_if = "Option::is_none")]
    codepoint: Option<char>,
}

impl From<&Diagnostic> for DiagnosticWire {
    fn from(d: &Diagnostic) -> Self {
        match d {
            Diagnostic::SourceContainsPua {
                codepoint, span, ..
            } => Self {
                kind: "source_contains_pua",
                span: (*span).into(),
                codepoint: Some(*codepoint),
            },
            Diagnostic::UnclosedBracket { span, .. } => Self {
                kind: "unclosed_bracket",
                span: (*span).into(),
                codepoint: None,
            },
            Diagnostic::UnmatchedClose { span, .. } => Self {
                kind: "unmatched_close",
                span: (*span).into(),
                codepoint: None,
            },
            Diagnostic::ResidualAnnotationMarker { span, .. } => Self {
                kind: "residual_annotation_marker",
                span: (*span).into(),
                codepoint: None,
            },
            Diagnostic::UnregisteredSentinel {
                codepoint, span, ..
            } => Self {
                kind: "unregistered_sentinel",
                span: (*span).into(),
                codepoint: Some(*codepoint),
            },
            Diagnostic::RegistryOutOfOrder { span, .. } => Self {
                kind: "registry_out_of_order",
                span: (*span).into(),
                codepoint: None,
            },
            Diagnostic::RegistryPositionMismatch { expected, span, .. } => Self {
                kind: "registry_position_mismatch",
                span: (*span).into(),
                codepoint: Some(*expected),
            },
            _ => Self {
                kind: "unknown",
                span: Span::new(0, 0).into(),
                codepoint: None,
            },
        }
    }
}

#[derive(Serialize)]
struct NodeWire {
    kind: &'static str,
    span: SpanWire,
}

#[derive(Serialize)]
struct PairWire {
    kind: &'static str,
    open: SpanWire,
    close: SpanWire,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    #[test]
    fn schema_version_is_one() {
        assert_eq!(SCHEMA_VERSION, 1);
    }

    #[test]
    fn empty_diagnostics_round_trip_envelope() {
        let json = serialize_diagnostics(&[]);
        assert_eq!(json, r#"{"schema_version":1,"data":[]}"#);
    }

    #[test]
    fn empty_nodes_round_trip_envelope() {
        let doc = Document::new("plain");
        let tree = doc.parse();
        let json = serialize_nodes(&tree);
        assert_eq!(json, r#"{"schema_version":1,"data":[]}"#);
    }

    #[test]
    fn empty_pairs_round_trip_envelope() {
        let doc = Document::new("plain");
        let tree = doc.parse();
        let json = serialize_pairs(&tree);
        assert_eq!(json, r#"{"schema_version":1,"data":[]}"#);
    }

    #[test]
    fn pua_collision_serialises_as_warning_kind() {
        let doc = Document::new("abc\u{E001}def");
        let tree = doc.parse();
        let json = serialize_diagnostics(tree.diagnostics());
        assert!(json.contains(r#""schema_version":1"#));
        assert!(json.contains(r#""kind":"source_contains_pua""#));
        assert!(json.contains(r#""codepoint":"""#) || json.contains(r#""codepoint":""#));
    }

    #[test]
    fn ruby_serialises_with_kind_ruby_in_nodes() {
        let doc = Document::new("｜青梅《おうめ》");
        let tree = doc.parse();
        let json = serialize_nodes(&tree);
        assert!(json.contains(r#""kind":"ruby""#));
        assert!(json.contains(r#""schema_version":1"#));
    }

    #[test]
    fn ruby_serialises_in_pairs() {
        let doc = Document::new("｜青梅《おうめ》");
        let tree = doc.parse();
        let json = serialize_pairs(&tree);
        assert!(json.contains(r#""kind":"ruby""#));
        assert!(json.contains(r#""open":"#));
        assert!(json.contains(r#""close":"#));
    }

    #[test]
    fn pair_kind_str_covers_all_known_kinds() {
        assert_eq!(pair_kind_str(PairKind::Bracket), "bracket");
        assert_eq!(pair_kind_str(PairKind::Ruby), "ruby");
        assert_eq!(pair_kind_str(PairKind::DoubleRuby), "doubleRuby");
        assert_eq!(pair_kind_str(PairKind::Tortoise), "tortoise");
        assert_eq!(pair_kind_str(PairKind::Quote), "quote");
    }
}
