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
//! Every JSON envelope has the shape
//!
//! ```json
//! { "schemaVersion": 1, "data": [ /* …entries… */ ] }
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
//! Both [`crate::Diagnostic`] and [`crate::Node`] are
//! `#[non_exhaustive]` upstream so the library can add variants in
//! minor releases. The wire format protects callers by:
//!
//! - falling back to `kind: "unknown"` for unrecognised variants, and
//! - bumping [`SCHEMA_VERSION`] when new variants land in the wire
//!   (so a client that branches on the version can react before
//!   `"unknown"` shows up in production traffic).

use serde::Serialize;

use crate::encoding::gaiji::{self, find_span, gaiji_resolutions, resolve_at};
use crate::{DiagnosticSource, Severity, Tree};

/// Wire-format schema version. Bumped on any breaking change to the
/// serialised shape (variant additions, field renames, envelope
/// changes).
pub const SCHEMA_VERSION: u32 = 1;

/// Project a slice of [`crate::Diagnostic`] into a `{ schema_version, data }`
/// JSON envelope. Every entry has the shape
/// `{ kind, span: { start, end }, codepoint? }`.
///
/// Empty input → `{"schemaVersion":1,"data":[]}`.
#[must_use]
pub fn diagnostics(diagnostics: &[crate::Diagnostic]) -> String {
    serialize_envelope(&diagnostic_entries(diagnostics))
}

/// The structured `Diagnostic` records that back `diagnostics()` —
/// prefer this to re-parsing the JSON when a caller needs the values
/// directly (e.g. a Wasm binding building JS objects).
#[must_use]
pub fn diagnostic_entries(diagnostics: &[crate::Diagnostic]) -> Vec<Diagnostic> {
    diagnostics.iter().map(Diagnostic::from).collect()
}

/// Project an [`Tree`]'s source-keyed node side-table into a
/// `{ schema_version, data }` JSON envelope.
///
/// Every entry has the shape `{ kind, span: { start, end } }`,
/// source-coordinate, sorted by `span.start`. Empty parse →
/// `{"schemaVersion":1,"data":[]}`.
#[must_use]
pub fn nodes(tree: &Tree<'_>) -> String {
    serialize_envelope(&node_entries(tree))
}

/// The structured `Node` records that back `nodes()` — prefer this to
/// re-parsing the JSON when a caller needs the values directly.
#[must_use]
pub fn node_entries(tree: &Tree<'_>) -> Vec<Node> {
    tree.source_nodes()
        .iter()
        .map(|sn| Node {
            kind: sn.node.kind().as_json_tag(),
            span: sn.source_span.into(),
        })
        .collect()
}

/// Project an [`Tree`]'s pair table into a
/// `{ schema_version, data }` JSON envelope. Every entry has the shape
/// `{ kind, open: { start, end }, close: { start, end } }`.
///
/// One entry per matched open/close pair; unmatched closes and
/// unclosed opens are excluded (they have no partner span and would
/// only confuse editor surfaces). Useful for LSP requests like
/// `textDocument/linkedEditingRange` and
/// `textDocument/documentHighlight`.
///
/// Empty parse → `{"schemaVersion":1,"data":[]}`.
#[must_use]
pub fn pairs(tree: &Tree<'_>) -> String {
    serialize_envelope(&pair_entries(tree))
}

/// The structured `Pair` records that back `pairs()` — prefer this to
/// re-parsing the JSON when a caller needs the values directly.
#[must_use]
pub fn pair_entries(tree: &Tree<'_>) -> Vec<Pair> {
    tree.pairs()
        .iter()
        .map(|link| Pair {
            kind: link.kind.as_json_tag(),
            open: link.open.into(),
            close: link.close.into(),
        })
        .collect()
}

/// Project an [`Tree`]'s container open/close pair table into a
/// `{ schema_version, data }` JSON envelope.
///
/// Each entry has the shape
/// `{ kind, open: { offset }, close: { offset } }` where `kind` is
/// the [`crate::RegionFormat`] discriminant (one of `"indent"` /
/// `"warichu"` / `"framed"` / `"alignEnd"`) and the offsets are
/// **normalized-coordinate** byte positions that index the PUA
/// sentinel positions — not the source span the user wrote.
///
/// Coordinate-system distinction matters: editor surfaces that want
/// source-coordinate container pairs must translate through
/// [`Tree::source_nodes`].
///
/// Empty parse → `{"schemaVersion":1,"data":[]}`.
#[must_use]
pub fn container_pairs(tree: &Tree<'_>) -> String {
    serialize_envelope(&container_pair_entries(tree))
}

/// The structured `ContainerPair` records that back
/// `container_pairs()` — prefer this to re-parsing the JSON when a caller
/// needs the values directly.
#[must_use]
pub fn container_pair_entries(tree: &Tree<'_>) -> Vec<ContainerPair> {
    tree.container_pairs()
        .iter()
        .map(|pair| ContainerPair {
            kind: pair.kind.as_json_tag(),
            open: Offset {
                offset: pair.open.get(),
            },
            close: Offset {
                offset: pair.close.get(),
            },
        })
        .collect()
}

/// Project the canonical slug catalogue ([`crate::SLUGS`]) into a
/// `{ schema_version, data }` JSON envelope.
///
/// Each entry has the shape `{ canonical, family, accepts_param, doc,
/// partner }`: `family` is the camelCase form of the
/// [`crate::SlugFamily`] variant, `partner` is `null` for non-paired
/// families. A static catalogue, independent of any parse — it powers
/// editor completion menus for `［＃…］` annotations without
/// re-implementing the table per driver (`aozora-wasm` / `aozora-py`
/// both call this).
#[must_use]
pub fn slugs() -> String {
    serialize_envelope(&slug_entries())
}

/// The structured `Slug` records that back `slugs()` — prefer this to
/// re-parsing the JSON when a caller needs the catalogue directly.
#[must_use]
pub fn slug_entries() -> Vec<Slug> {
    crate::SLUGS
        .iter()
        .map(|s| Slug {
            canonical: s.canonical,
            family: s.family.as_json_tag(),
            accepts_param: s.accepts_param,
            doc: s.doc,
            partner: s.partner,
        })
        .collect()
}

/// Project resolved `※［＃…］` gaiji references from `source` into a
/// `{ schema_version, data }` JSON envelope.
///
/// Each entry is
/// `{ span: { start, end }, description, mencode, codepoint, resolved }`
/// in source-byte coordinates; `mencode` / `codepoint` / `resolved` are
/// `null` when absent or unresolved. Walks the source once — `O(source)`.
///
/// Powers inlay-hint UIs (`→GLYPH` after each reference) and batch gaiji
/// audits. The scan + resolution are the single authority in
/// [`crate::encoding::gaiji`]; this is only their wire projection.
///
/// Empty / gaiji-free source → `{"schemaVersion":1,"data":[]}`.
#[must_use]
pub fn gaiji(source: &str) -> String {
    serialize_envelope(&gaiji_entries(source))
}

/// The structured `GaijiResolution` records that back `gaiji()` —
/// prefer this to re-parsing the JSON when a caller needs the values
/// directly.
#[must_use]
pub fn gaiji_entries(source: &str) -> Vec<GaijiResolution> {
    gaiji_resolutions(source)
        .into_iter()
        .map(Into::into)
        .collect()
}

/// Resolve the gaiji reference at `byte_offset` (cursor-local).
///
/// Serialises the single
/// `{ span, description, mencode, codepoint, resolved }` object — or the
/// literal `"null"` when the offset is not inside a `※［＃…］` span.
///
/// For editor cursor-hover: the scan is bounded to a window around the
/// cursor, so cost is independent of document size (unlike
/// [`gaiji()`], which walks the whole source).
#[must_use]
pub fn gaiji_at(source: &str, byte_offset: usize) -> String {
    find_span(source, byte_offset)
        .and_then(|(start, end)| resolve_at(source, start, end))
        .map_or_else(
            || "null".to_owned(),
            |g| {
                serde_json::to_string(&GaijiResolution::from(g))
                    .unwrap_or_else(|_| "null".to_owned())
            },
        )
}

// ────────────────────────────────────────────────────────────────────
// Internal: envelope + wire structs
// ────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Envelope<'a, T> {
    schema_version: u32,
    data: &'a [T],
}

// ────────────────────────────────────────────────────────────────────
// JSON Schema introspection
// ────────────────────────────────────────────────────────────────────

/// JSON Schema (draft 2020-12) describing the
/// [`diagnostics`] envelope output.
///
/// Schema-feature only. Used by `xtask schema dump` to commit the
/// schema artefact under `crates/aozora-book/src/json/`, and by the
/// `aozora schema` CLI subcommand for ad-hoc introspection.
#[cfg(feature = "schema")]
#[must_use]
pub fn schema_diagnostics() -> serde_json::Value {
    envelope_schema(
        "AozoraDiagnosticsEnvelope",
        "Envelope returned by aozora::json::diagnostics.",
        schemars::schema_for!(Diagnostic),
    )
}

/// JSON Schema for the [`nodes`] envelope output.
#[cfg(feature = "schema")]
#[must_use]
pub fn schema_nodes() -> serde_json::Value {
    envelope_schema(
        "AozoraNodesEnvelope",
        "Envelope returned by aozora::json::nodes.",
        schemars::schema_for!(Node),
    )
}

/// JSON Schema for the [`pairs`] envelope output.
#[cfg(feature = "schema")]
#[must_use]
pub fn schema_pairs() -> serde_json::Value {
    envelope_schema(
        "AozoraPairsEnvelope",
        "Envelope returned by aozora::json::pairs.",
        schemars::schema_for!(Pair),
    )
}

/// JSON Schema for the [`container_pairs`] envelope output.
#[cfg(feature = "schema")]
#[must_use]
pub fn schema_container_pairs() -> serde_json::Value {
    envelope_schema(
        "AozoraContainerPairsEnvelope",
        "Envelope returned by aozora::json::container_pairs.",
        schemars::schema_for!(ContainerPair),
    )
}

/// Wrap the per-entry schema in the canonical
/// `{schema_version, data: […]}` envelope. The envelope shape is
/// shared by all four wire functions; only the inner item schema
/// varies.
#[cfg(feature = "schema")]
fn envelope_schema(
    title: &str,
    description: &str,
    item_schema: schemars::Schema,
) -> serde_json::Value {
    // `schema_for!(ItemWire)` returns a self-contained document: its
    // shared sub-types (e.g. `Span`) live under a root `$defs` and
    // are referenced as `#/$defs/…`. Embedding it verbatim as `items`
    // would bury that `$defs` under `properties/data/items`, leaving the
    // `#/$defs/…` refs — which resolve against the *document* root —
    // dangling, so strict resolvers (quicktype, and any other consumer
    // of the published schema) reject it. Hoist the item schema's
    // `$defs` to the envelope root and drop its redundant per-item
    // `$schema` dialect marker so the refs resolve against the root.
    let mut item = item_schema.to_value();
    let defs = item.as_object_mut().and_then(|obj| {
        obj.remove("$schema");
        obj.remove("$defs")
    });
    let mut root = serde_json::json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": title,
        "description": description,
        "type": "object",
        "additionalProperties": false,
        "required": ["schemaVersion", "data"],
        "properties": {
            "schemaVersion": {
                "description": "Wire schema version. See aozora::json::SCHEMA_VERSION.",
                "type": "integer",
                "const": SCHEMA_VERSION,
            },
            "data": {
                "description": "Per-entry payload array; one item per emitted diagnostic / node / pair.",
                "type": "array",
                "items": item,
            },
        },
    });
    if let Some(defs) = defs {
        root.as_object_mut()
            .expect("envelope root is a JSON object literal")
            .insert("$defs".to_owned(), defs);
    }
    root
}

fn serialize_envelope<T: Serialize>(data: &[T]) -> String {
    let env = Envelope {
        schema_version: SCHEMA_VERSION,
        data,
    };
    serde_json::to_string(&env)
        .unwrap_or_else(|_| format!(r#"{{"schemaVersion":{SCHEMA_VERSION},"data":[]}}"#))
}

/// One half-open `[start, end)` byte span in a wire envelope.
#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Span {
    start: u32,
    end: u32,
}

impl From<crate::Span> for Span {
    fn from(s: crate::Span) -> Self {
        Self {
            start: s.start,
            end: s.end,
        }
    }
}

/// One `diagnostics` envelope entry — a projected [`crate::Diagnostic`].
#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Diagnostic {
    kind: &'static str,
    severity: &'static str,
    source: &'static str,
    span: Span,
    #[serde(skip_serializing_if = "Option::is_none")]
    codepoint: Option<char>,
}

impl From<&crate::Diagnostic> for Diagnostic {
    fn from(d: &crate::Diagnostic) -> Self {
        // Pull the codepoint payload off the variants that carry one.
        // The accessors collapse the Internal/Source distinction for
        // severity/source/code; the codepoint is the only payload that
        // survives variant-by-variant.
        let codepoint = match d {
            crate::Diagnostic::SourceContainsPua { codepoint, .. } => Some(*codepoint),
            _ => None,
        };
        // Strip the `aozora::lex::` / `aozora::internal` prefix so the
        // wire `kind` stays terse — this matches the prior wire layout
        // where the tag was the trailing token (e.g. "source_contains_pua",
        // "unclosed_bracket"). Internal codes get the same trailing-token
        // treatment so the wire `kind` is uniform across the user-facing
        // and internal axes; consumers that need the full namespaced ID
        // can still rely on `Diagnostic::code()`.
        let kind = d.code().rsplit("::").next().unwrap_or("unknown");
        Self {
            kind,
            severity: severity_str(d.severity()),
            source: source_str(d.source()),
            span: d.span().into(),
            codepoint,
        }
    }
}

const fn severity_str(s: Severity) -> &'static str {
    // `Severity` is `#[non_exhaustive]` upstream — the wildcard arm
    // covers any future variant by defaulting to "error" so consumers
    // err on the side of surfacing it until they upgrade.
    match s {
        Severity::Warning => "warning",
        Severity::Note => "note",
        Severity::Error | _ => "error",
    }
}

const fn source_str(s: DiagnosticSource) -> &'static str {
    // `DiagnosticSource` is `#[non_exhaustive]` upstream — the
    // wildcard arm covers any future variant by defaulting to
    // "internal" so consumers filtering library-bug diagnostics
    // catch it until they upgrade.
    match s {
        DiagnosticSource::Source => "source",
        DiagnosticSource::Internal | _ => "internal",
    }
}

/// One `nodes` envelope entry — a classified node span in source coords.
#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Node {
    kind: &'static str,
    span: Span,
}

/// One `pairs` envelope entry — a matched bracket pair.
#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Pair {
    kind: &'static str,
    open: Span,
    close: Span,
}

/// One `container_pairs` envelope entry — a paired container, open/close
/// in normalized coordinates.
#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ContainerPair {
    kind: &'static str,
    open: Offset,
    close: Offset,
}

/// A single byte offset (a `container_pairs` open/close in normalized
/// coordinates).
#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Offset {
    offset: u32,
}

/// One `slugs` envelope entry — a row of the annotation slug catalogue.
#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Slug {
    canonical: &'static str,
    family: &'static str,
    accepts_param: bool,
    doc: &'static str,
    // No `skip_serializing_if`: non-paired families emit `partner:null`
    // (byte-identical to the prior aozora-wasm `slugs_json` shape).
    partner: Option<&'static str>,
}

/// Source-byte span carried by [`GaijiResolution`].
///
/// Distinct from [`Span`] (whose `u32` fields cover sanitized-source
/// spans): gaiji offsets are raw `usize` byte positions into the original
/// source, kept as-is to stay byte-identical to the prior aozora-wasm
/// projection.
#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ByteSpan {
    start: usize,
    end: usize,
}

/// One `gaiji` envelope entry — a resolved `※［＃…］` reference.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GaijiResolution {
    span: ByteSpan,
    description: String,
    // Nulls (not skipped) so the shape is fixed across entries.
    mencode: Option<String>,
    codepoint: Option<u32>,
    resolved: Option<String>,
}

impl From<gaiji::GaijiResolution> for GaijiResolution {
    fn from(g: gaiji::GaijiResolution) -> Self {
        Self {
            span: ByteSpan {
                start: g.start,
                end: g.end,
            },
            description: g.description,
            mencode: g.mencode,
            codepoint: g.codepoint,
            resolved: g.resolved,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    #[test]
    fn slugs_envelope_lists_catalogue_with_known_families() {
        let json = slugs();
        assert!(json.contains(r#""schemaVersion":1"#));
        assert!(json.contains(r#""canonical":"#));
        assert!(json.contains(r#""family":"#));
        // Guard against the silent `_ => "unknown"` degrade: every
        // shipped slug must map to an explicit camelCase family.
        assert!(
            !json.contains(r#""family":"unknown""#),
            "shipped catalogue leaked an unknown family: {json}"
        );
    }

    #[test]
    fn gaiji_resolutions_empty_envelope_for_plain_text() {
        assert_eq!(gaiji("no gaiji here"), r#"{"schemaVersion":1,"data":[]}"#);
    }

    #[test]
    fn gaiji_resolutions_emits_resolved_entry_in_source_coords() {
        let json = gaiji("※［＃「々」］");
        assert!(json.contains(r#""schemaVersion":1"#));
        assert!(
            json.contains(r#""span":{"start":0,"end":21}"#),
            "json: {json}"
        );
        assert!(json.contains(r#""description":"々""#), "json: {json}");
        assert!(json.contains(r#""resolved":"々""#), "json: {json}");
        // Unresolved/absent fields serialise as null, not skipped.
        assert!(json.contains(r#""mencode":null"#), "json: {json}");
    }

    #[test]
    fn gaiji_resolution_at_returns_object_inside_span_else_null() {
        let src = "あ※［＃「々」］い";
        let inside = src.find('※').unwrap() + "※".len();
        let at = gaiji_at(src, inside);
        assert!(at.contains(r#""description":"々""#), "{at}");
        assert!(at.contains(r#""resolved":"々""#), "{at}");
        // A cursor outside any reference resolves to the literal "null".
        assert_eq!(gaiji_at(src, 0), "null");
    }

    #[test]
    fn schema_version_is_one() {
        assert_eq!(SCHEMA_VERSION, 1);
    }

    #[test]
    fn empty_diagnostics_round_trip_envelope() {
        let json = diagnostics(&[]);
        assert_eq!(json, r#"{"schemaVersion":1,"data":[]}"#);
    }

    #[test]
    fn empty_nodes_round_trip_envelope() {
        let doc = Document::new("plain");
        let tree = doc.parse();
        let json = nodes(&tree);
        assert_eq!(json, r#"{"schemaVersion":1,"data":[]}"#);
    }

    #[test]
    fn empty_pairs_round_trip_envelope() {
        let doc = Document::new("plain");
        let tree = doc.parse();
        let json = pairs(&tree);
        assert_eq!(json, r#"{"schemaVersion":1,"data":[]}"#);
    }

    #[test]
    fn pua_collision_serialises_as_warning_kind() {
        let doc = Document::new("abc\u{E001}def");
        let tree = doc.parse();
        let json = diagnostics(tree.diagnostics());
        assert!(json.contains(r#""schemaVersion":1"#));
        assert!(json.contains(r#""kind":"source_contains_pua""#));
        assert!(json.contains(r#""codepoint":"""#) || json.contains(r#""codepoint":""#));
    }

    #[test]
    fn ruby_serialises_with_kind_ruby_in_nodes() {
        let doc = Document::new("｜青梅《おうめ》");
        let tree = doc.parse();
        let json = nodes(&tree);
        assert!(json.contains(r#""kind":"ruby""#));
        assert!(json.contains(r#""schemaVersion":1"#));
    }

    #[test]
    fn ruby_serialises_in_pairs() {
        let doc = Document::new("｜青梅《おうめ》");
        let tree = doc.parse();
        let json = pairs(&tree);
        assert!(json.contains(r#""kind":"ruby""#));
        assert!(json.contains(r#""open":"#));
        assert!(json.contains(r#""close":"#));
    }

    #[test]
    fn pair_kind_camel_case_covers_all_known_kinds() {
        use crate::PairKind;
        assert_eq!(PairKind::Bracket.as_json_tag(), "bracket");
        assert_eq!(PairKind::Ruby.as_json_tag(), "ruby");
        assert_eq!(PairKind::AngleQuote.as_json_tag(), "angleQuote");
        assert_eq!(PairKind::Tortoise.as_json_tag(), "tortoise");
        assert_eq!(PairKind::Quote.as_json_tag(), "quote");
    }

    #[test]
    fn container_kind_wire_tags_via_as_json_tag() {
        use aozora_syntax::{BoutenKind, BoutenPosition, RegionFormat};
        // `RegionFormat::as_json_tag` is the single authority on the
        // container-pairs wire tag (no `_ => "unknown"` fallback —
        // exhaustiveness is enforced in aozora-syntax). The scope-specific
        // `boutenRange` / `combineUprightRange` strings are preserved verbatim
        // so SCHEMA_VERSION=1 stays byte-stable.
        assert_eq!(RegionFormat::Bold { padded: false }.as_json_tag(), "bold");
        assert_eq!(RegionFormat::Bold { padded: true }.as_json_tag(), "bold");
        assert_eq!(
            RegionFormat::Italic { padded: false }.as_json_tag(),
            "italic"
        );
        assert_eq!(
            RegionFormat::Italic { padded: true }.as_json_tag(),
            "italic"
        );
        assert_eq!(
            RegionFormat::Bouten {
                kind: BoutenKind::Goma,
                position: BoutenPosition::Right,
            }
            .as_json_tag(),
            "boutenRange"
        );
        assert_eq!(
            RegionFormat::CombineUpright.as_json_tag(),
            "combineUprightRange"
        );
    }
}
