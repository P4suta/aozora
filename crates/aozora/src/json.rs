//! Serializes parser projections as versioned JSON envelopes.
//! Each envelope contains `schemaVersion` and a `data` array.
//! Check [`SCHEMA_VERSION`] before interpreting an envelope.

use serde::Serialize;

use crate::encoding::gaiji;
use crate::spec::SLUGS;
use crate::{DiagnosticSource, Severity, Snapshot};

/// Current JSON wire-format version.
pub const SCHEMA_VERSION: u32 = 3;

/// Serialize diagnostics as [`Diagnostic`] entries in a versioned envelope.
#[must_use]
pub fn diagnostics(diagnostics: &[crate::Diagnostic]) -> String {
    serialize_envelope(&diagnostic_entries(diagnostics))
}

/// Return the records serialized by [`diagnostics`].
#[must_use]
pub fn diagnostic_entries(diagnostics: &[crate::Diagnostic]) -> Vec<Diagnostic> {
    diagnostics.iter().map(Diagnostic::from).collect()
}

/// Serialize snapshot nodes in source order as [`Node`] entries.
#[must_use]
pub fn nodes(snapshot: &Snapshot) -> String {
    serialize_envelope(&node_entries(snapshot))
}

/// Return the records serialized by [`nodes`].
#[must_use]
pub fn node_entries(snapshot: &Snapshot) -> Vec<Node> {
    snapshot
        .nodes()
        .iter()
        .map(|node| Node {
            kind: node.kind().as_json_tag(),
            span: node.span().into(),
        })
        .collect()
}

/// Serialize matched delimiters as [`Pair`] entries.
///
/// Excludes unmatched delimiters.
#[must_use]
pub fn pairs(snapshot: &Snapshot) -> String {
    serialize_envelope(&pair_entries(snapshot))
}

/// Return the records serialized by [`pairs`].
#[must_use]
pub fn pair_entries(snapshot: &Snapshot) -> Vec<Pair> {
    snapshot
        .pairs()
        .iter()
        .map(|link| Pair {
            kind: link.kind.as_json_tag(),
            open: link.open.into(),
            close: link.close.into(),
        })
        .collect()
}

/// Serialize matched container delimiters as [`ContainerPair`] entries.
#[must_use]
pub fn container_pairs(snapshot: &Snapshot) -> String {
    serialize_envelope(&container_pair_entries(snapshot))
}

/// Return the records serialized by [`container_pairs`].
#[must_use]
pub fn container_pair_entries(snapshot: &Snapshot) -> Vec<ContainerPair> {
    snapshot
        .container_pairs()
        .iter()
        .map(|pair| ContainerPair {
            kind: pair.kind().as_str(),
            open: pair.open().into(),
            close: pair.close().into(),
        })
        .collect()
}

/// Serialize the notation catalogue as [`Slug`] entries.
#[must_use]
pub fn slugs() -> String {
    serialize_envelope(&slug_entries())
}

/// Return the records serialized by [`slugs`].
#[must_use]
pub fn slug_entries() -> Vec<Slug> {
    SLUGS
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

/// Serialize snapshot gaiji references as [`GaijiResolution`] entries.
#[must_use]
pub fn gaiji(snapshot: &Snapshot) -> String {
    serialize_envelope(&gaiji_entries(snapshot))
}

/// Return the records serialized by [`gaiji`].
#[must_use]
pub fn gaiji_entries(snapshot: &Snapshot) -> Vec<GaijiResolution> {
    snapshot
        .gaiji_resolutions()
        .iter()
        .cloned()
        .map(Into::into)
        .collect()
}

/// Return a gaiji wire record at a source byte offset, or `None` if none covers it.
#[must_use]
pub fn gaiji_entry_at(snapshot: &Snapshot, byte_offset: usize) -> Option<GaijiResolution> {
    snapshot
        .gaiji_resolution_at(byte_offset)
        .cloned()
        .map(Into::into)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Envelope<'a, T> {
    schema_version: u32,
    data: &'a [T],
}

/// JSON Schema draft 2020-12 for the [`diagnostics`] envelope.
#[cfg(feature = "schema")]
#[cfg_attr(docsrs, doc(cfg(feature = "schema")))]
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
#[cfg_attr(docsrs, doc(cfg(feature = "schema")))]
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
#[cfg_attr(docsrs, doc(cfg(feature = "schema")))]
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
#[cfg_attr(docsrs, doc(cfg(feature = "schema")))]
#[must_use]
pub fn schema_container_pairs() -> serde_json::Value {
    envelope_schema(
        "AozoraContainerPairsEnvelope",
        "Envelope returned by aozora::json::container_pairs.",
        schemars::schema_for!(ContainerPair),
    )
}

/// JSON Schema for the [`gaiji`] envelope output.
#[cfg(feature = "schema")]
#[cfg_attr(docsrs, doc(cfg(feature = "schema")))]
#[must_use]
pub fn schema_gaiji() -> serde_json::Value {
    envelope_schema(
        "AozoraGaijiEnvelope",
        "Envelope returned by aozora::json::gaiji.",
        schemars::schema_for!(GaijiResolution),
    )
}

/// JSON Schema for the [`slugs`] envelope output.
#[cfg(feature = "schema")]
#[cfg_attr(docsrs, doc(cfg(feature = "schema")))]
#[must_use]
pub fn schema_slugs() -> serde_json::Value {
    envelope_schema(
        "AozoraSlugsEnvelope",
        "Envelope returned by aozora::json::slugs.",
        schemars::schema_for!(Slug),
    )
}

#[cfg(feature = "schema")]
const JSON_SCHEMA_DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

#[cfg(feature = "schema")]
#[expect(
    clippy::expect_used,
    reason = "the envelope root is constructed from an object literal"
)]
fn envelope_schema(
    title: &str,
    description: &str,
    item_schema: schemars::Schema,
) -> serde_json::Value {
    let mut item = item_schema.to_value();
    let defs = item.as_object_mut().and_then(|obj| {
        obj.remove("$schema");
        obj.remove("$defs")
    });
    let mut root = serde_json::json!({
        "$schema": JSON_SCHEMA_DIALECT,
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
    match serde_json::to_string(&env) {
        Ok(json) => json,
        Err(error) => unreachable!("wire envelope serialization failed: {error}"),
    }
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
    codepoint: Option<u32>,
}

impl From<&crate::Diagnostic> for Diagnostic {
    fn from(d: &crate::Diagnostic) -> Self {
        let codepoint = match d {
            crate::Diagnostic::SourceContainsPua { codepoint, .. } => Some(u32::from(*codepoint)),
            _ => None,
        };
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
    match s {
        Severity::Warning => "warning",
        Severity::Note => "note",
        Severity::Error => "error",
    }
}

const fn source_str(s: DiagnosticSource) -> &'static str {
    match s {
        DiagnosticSource::Source => "source",
        DiagnosticSource::Internal => "internal",
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

/// One `container_pairs` envelope entry.
#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ContainerPair {
    kind: &'static str,
    open: Span,
    close: Span,
}

/// One `slugs` envelope entry — a row of the annotation slug catalogue.
#[derive(Debug, Clone, Copy, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Slug {
    canonical: &'static str,
    family: &'static str,
    accepts_param: bool,
    doc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    partner: Option<&'static str>,
}

/// One `gaiji` envelope entry — a resolved `※［＃…］` reference.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct GaijiResolution {
    span: Span,
    description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    mencode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    codepoint: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolved: Option<String>,
}

impl From<gaiji::GaijiResolution> for GaijiResolution {
    fn from(g: gaiji::GaijiResolution) -> Self {
        Self {
            span: g.span().into(),
            description: g.description().to_owned(),
            mencode: g.mencode().map(str::to_owned),
            codepoint: g.codepoint(),
            resolved: g.resolved().map(str::to_owned),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;
    use serde::ser::Error as _;

    struct RejectSerialization;

    impl Serialize for RejectSerialization {
        fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Err(S::Error::custom("rejected"))
        }
    }

    fn empty_envelope() -> String {
        format!(r#"{{"schemaVersion":{SCHEMA_VERSION},"data":[]}}"#)
    }

    fn schema_marker() -> String {
        format!(r#""schemaVersion":{SCHEMA_VERSION}"#)
    }

    #[test]
    fn slugs_envelope_lists_catalogue_with_known_families() {
        let json = slugs();
        assert!(json.contains(&schema_marker()));
        assert!(json.contains(r#""canonical":"#));
        assert!(json.contains(r#""family":"#));
        assert!(
            !json.contains(r#""family":"unknown""#),
            "shipped catalogue leaked an unknown family: {json}"
        );
    }

    #[test]
    fn gaiji_resolutions_empty_envelope_for_plain_text() {
        let snapshot = Document::new("no gaiji here").snapshot();
        assert_eq!(gaiji(&snapshot), empty_envelope());
    }

    #[test]
    fn gaiji_resolutions_emits_resolved_entry_in_source_coords() {
        let snapshot = Document::new("※［＃「々」］").snapshot();
        let json = gaiji(&snapshot);
        assert!(json.contains(&schema_marker()));
        assert!(
            json.contains(r#""span":{"start":0,"end":21}"#),
            "json: {json}"
        );
        assert!(json.contains(r#""description":"々""#), "json: {json}");
        assert!(json.contains(r#""resolved":"々""#), "json: {json}");
        assert!(!json.contains(r#""mencode""#), "json: {json}");
    }

    #[test]
    fn gaiji_entry_at_resolves_only_inside_span() {
        let src = "あ※［＃「々」］い";
        let snapshot = Document::new(src).snapshot();
        let inside = src.find('※').unwrap() + "※".len();
        let at = gaiji_entry_at(&snapshot, inside).expect("inside gaiji");
        assert_eq!(at.description, "々");
        assert_eq!(at.resolved.as_deref(), Some("々"));
        assert!(gaiji_entry_at(&snapshot, 0).is_none());
    }

    #[test]
    fn schema_version_matches_the_wire_authority() {
        assert_eq!(SCHEMA_VERSION, 3);
    }

    #[test]
    fn empty_diagnostics_round_trip_envelope() {
        let json = diagnostics(&[]);
        assert_eq!(json, empty_envelope());
    }

    #[test]
    #[should_panic(expected = "wire envelope serialization failed: rejected")]
    fn serialization_failure_is_not_reported_as_an_empty_envelope() {
        drop(serialize_envelope(&[RejectSerialization]));
    }

    #[test]
    fn empty_nodes_round_trip_envelope() {
        let doc = Document::new("plain");
        let tree = doc.snapshot();
        let json = nodes(&tree);
        assert_eq!(json, empty_envelope());
    }

    #[test]
    fn empty_pairs_round_trip_envelope() {
        let doc = Document::new("plain");
        let tree = doc.snapshot();
        let json = pairs(&tree);
        assert_eq!(json, empty_envelope());
    }

    #[test]
    fn pua_collision_serialises_as_warning_kind() {
        let doc = Document::new("abc\u{E001}def");
        let tree = doc.snapshot();
        let json = diagnostics(tree.diagnostics());
        assert!(json.contains(&schema_marker()));
        assert!(json.contains(r#""kind":"source_contains_pua""#));
        assert!(json.contains(&format!(r#""codepoint":{}"#, '\u{E001}' as u32)));
    }

    #[test]
    fn ruby_serialises_with_kind_ruby_in_nodes() {
        let doc = Document::new("｜青梅《おうめ》");
        let tree = doc.snapshot();
        let json = nodes(&tree);
        assert!(json.contains(r#""kind":"ruby""#));
        assert!(json.contains(&schema_marker()));
    }

    #[test]
    fn ruby_serialises_in_pairs() {
        let doc = Document::new("｜青梅《おうめ》");
        let tree = doc.snapshot();
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
    fn container_pair_entries_projects_indent_open_close_offsets() {
        let doc = Document::new("［＃ここから2字下げ］\n本文\n［＃ここで字下げ終わり］\n");
        let tree = doc.snapshot();
        let entries = container_pair_entries(&tree);
        assert_eq!(
            entries.len(),
            1,
            "expected exactly one indent container pair: {entries:?}"
        );
        let pair = &entries[0];
        assert_eq!(pair.kind, "indent", "container kind: {pair:?}");
        assert!(
            pair.open.start < pair.close.start,
            "open must precede close: {pair:?}"
        );
    }

    #[cfg(feature = "schema")]
    #[test]
    fn schema_fns_return_titled_envelope_not_null() {
        let cases = [
            ("AozoraDiagnosticsEnvelope", schema_diagnostics()),
            ("AozoraNodesEnvelope", schema_nodes()),
            ("AozoraPairsEnvelope", schema_pairs()),
            ("AozoraContainerPairsEnvelope", schema_container_pairs()),
            ("AozoraGaijiEnvelope", schema_gaiji()),
            ("AozoraSlugsEnvelope", schema_slugs()),
        ];
        for (title, value) in cases {
            assert_eq!(
                value["title"], title,
                "schema envelope title mismatch: {value}"
            );
            assert_eq!(
                value["type"], "object",
                "schema envelope must be an object: {value}"
            );
            assert_eq!(
                value["properties"]["schemaVersion"]["const"],
                serde_json::json!(SCHEMA_VERSION),
                "schemaVersion const must pin SCHEMA_VERSION: {value}"
            );
            assert_eq!(
                value["properties"]["data"]["type"], "array",
                "data property must be an array: {value}"
            );
        }
    }

    #[cfg(feature = "schema")]
    #[test]
    fn envelope_schema_wraps_item_in_versioned_envelope() {
        let value = envelope_schema(
            "CustomTitle",
            "Custom description.",
            schemars::schema_for!(Node),
        );
        assert_eq!(value["title"], "CustomTitle", "title: {value}");
        assert_eq!(
            value["description"], "Custom description.",
            "description: {value}"
        );
        assert_eq!(
            value["additionalProperties"], false,
            "closed shape: {value}"
        );
        assert_eq!(
            value["required"],
            serde_json::json!(["schemaVersion", "data"]),
            "required keys: {value}"
        );
        assert_eq!(
            value["properties"]["schemaVersion"]["const"],
            serde_json::json!(SCHEMA_VERSION),
            "schemaVersion const: {value}"
        );
    }

    #[test]
    fn container_kind_wire_tags_via_as_json_tag() {
        use crate::syntax::{BoutenKind, BoutenPosition, RegionFormat};
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
    }
}
