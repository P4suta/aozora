//! Pin the cross-driver wire format. Every driver (`aozora-ffi`,
//! `aozora-wasm`, `aozora-py`) calls into [`aozora::wire`] for JSON
//! projection; these tests fix that projection's byte-shape so future
//! drift is caught before drivers diverge.

#![cfg(feature = "wire")]

use aozora::{Document, wire};

/// The empty parse must serialise as the canonical empty envelope —
/// regardless of which projection function is called.
#[test]
fn empty_parse_serialises_to_canonical_envelope() {
    let doc = Document::new("plain");
    let tree = doc.parse();
    let canonical = r#"{"schema_version":1,"data":[]}"#;
    assert_eq!(wire::serialize_diagnostics(tree.diagnostics()), canonical);
    assert_eq!(wire::serialize_nodes(&tree), canonical);
    assert_eq!(wire::serialize_pairs(&tree), canonical);
}

/// Schema version is one. Bumped only when wire shape changes.
#[test]
fn schema_version_is_pinned_to_one() {
    assert_eq!(wire::SCHEMA_VERSION, 1);
}

/// PUA collision diagnostic shape, byte-pinned.
#[test]
fn pua_collision_diagnostic_byte_shape() {
    let doc = Document::new("a\u{E001}b");
    let tree = doc.parse();
    let json = wire::serialize_diagnostics(tree.diagnostics());
    // Envelope present.
    assert!(json.starts_with(r#"{"schema_version":1,"data":["#));
    assert!(json.ends_with("]}"));
    // Variant tag + severity / source axis + span shape.
    assert!(json.contains(r#""kind":"source_contains_pua""#));
    assert!(json.contains(r#""severity":"warning""#));
    assert!(json.contains(r#""source":"source""#));
    assert!(json.contains(r#""span":{"start":1,"end":4}"#));
    // codepoint field is present (escaped JSON form).
    assert!(json.contains(r#""codepoint":"#));
}

/// Severity / source axes are present and correctly classified.
#[test]
fn diagnostic_wire_has_severity_and_source_axes() {
    let doc = Document::new("a\u{E001}b");
    let tree = doc.parse();
    let json = wire::serialize_diagnostics(tree.diagnostics());
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    let entry = parsed
        .get("data")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .expect("at least one diagnostic");
    assert_eq!(
        entry.get("severity").and_then(|v| v.as_str()),
        Some("warning"),
        "PUA collision is a warning: {entry}"
    );
    assert_eq!(
        entry.get("source").and_then(|v| v.as_str()),
        Some("source"),
        "PUA collision is source-side: {entry}"
    );
}

/// Ruby span shape (nodes channel), byte-pinned.
#[test]
fn ruby_node_byte_shape() {
    let doc = Document::new("｜青梅《おうめ》");
    let tree = doc.parse();
    let json = wire::serialize_nodes(&tree);
    assert!(json.starts_with(r#"{"schema_version":1,"data":["#));
    assert!(json.contains(r#""kind":"ruby""#));
    assert!(json.contains(r#""span":{"start":"#));
}

/// Ruby pair shape (pairs channel), byte-pinned.
#[test]
fn ruby_pair_byte_shape() {
    let doc = Document::new("｜青梅《おうめ》");
    let tree = doc.parse();
    let json = wire::serialize_pairs(&tree);
    assert!(json.starts_with(r#"{"schema_version":1,"data":["#));
    assert!(json.contains(r#""kind":"ruby""#));
    assert!(json.contains(r#""open":{"start":"#));
    assert!(json.contains(r#""close":{"start":"#));
}

/// JSON parses round-trip through `serde_json` — proves valid output.
#[test]
fn all_three_channels_emit_valid_json() {
    let doc = Document::new("｜青梅《おうめ》abc\u{E001}def");
    let tree = doc.parse();
    for json in [
        wire::serialize_diagnostics(tree.diagnostics()),
        wire::serialize_nodes(&tree),
        wire::serialize_pairs(&tree),
    ] {
        let value: serde_json::Value =
            serde_json::from_str(&json).expect("wire output must be valid JSON");
        assert!(value.is_object(), "envelope must be JSON object");
        assert_eq!(
            value
                .get("schema_version")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert!(value.get("data").is_some_and(serde_json::Value::is_array));
    }
}
