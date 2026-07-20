//! Pin the cross-driver JSON format. Every driver (`aozora-ffi`,
//! `aozora-wasm`, `aozora-py`) calls into [`aozora::json`] for JSON
//! projection; these tests fix that projection's byte-shape so future
//! drift is caught before drivers diverge.

#![cfg(feature = "json")]

use aozora::json;

fn empty_envelope() -> String {
    format!(r#"{{"schemaVersion":{},"data":[]}}"#, json::SCHEMA_VERSION)
}

fn data_prefix() -> String {
    format!(r#"{{"schemaVersion":{},"data":["#, json::SCHEMA_VERSION)
}

/// The empty parse must serialise as the canonical empty envelope —
/// regardless of which projection function is called.
#[test]
fn empty_parse_serialises_to_canonical_envelope() {
    let doc = aozora::parse("plain").expect("source fits parser span limit");
    let tree = doc.snapshot();
    let canonical = empty_envelope();
    assert_eq!(json::diagnostics(tree.diagnostics()), canonical);
    assert_eq!(json::nodes(&tree), canonical);
    assert_eq!(json::pairs(&tree), canonical);
}

/// PUA collision diagnostic shape, byte-pinned.
#[test]
fn pua_collision_diagnostic_byte_shape() {
    let doc = aozora::parse("a\u{E001}b").expect("source fits parser span limit");
    let tree = doc.snapshot();
    let json = json::diagnostics(tree.diagnostics());
    // Envelope present.
    assert!(json.starts_with(&data_prefix()));
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
fn diagnostic_json_has_severity_and_source_axes() {
    let doc = aozora::parse("a\u{E001}b").expect("source fits parser span limit");
    let tree = doc.snapshot();
    let json = json::diagnostics(tree.diagnostics());
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
    let doc = aozora::parse("｜青梅《おうめ》").expect("source fits parser span limit");
    let tree = doc.snapshot();
    let json = json::nodes(&tree);
    assert!(json.starts_with(&data_prefix()));
    assert!(json.contains(r#""kind":"ruby""#));
    assert!(json.contains(r#""span":{"start":"#));
}

/// Ruby pair shape (pairs channel), byte-pinned.
#[test]
fn ruby_pair_byte_shape() {
    let doc = aozora::parse("｜青梅《おうめ》").expect("source fits parser span limit");
    let tree = doc.snapshot();
    let json = json::pairs(&tree);
    assert!(json.starts_with(&data_prefix()));
    assert!(json.contains(r#""kind":"ruby""#));
    assert!(json.contains(r#""open":{"start":"#));
    assert!(json.contains(r#""close":{"start":"#));
}

/// JSON parses round-trip through `serde_json` — proves valid output.
#[test]
fn all_three_channels_emit_valid_json() {
    let doc =
        aozora::parse("｜青梅《おうめ》abc\u{E001}def").expect("source fits parser span limit");
    let tree = doc.snapshot();
    for json in [
        json::diagnostics(tree.diagnostics()),
        json::nodes(&tree),
        json::pairs(&tree),
    ] {
        let value: serde_json::Value =
            serde_json::from_str(&json).expect("JSON output must be valid JSON");
        assert!(value.is_object(), "envelope must be JSON object");
        assert_eq!(
            value
                .get("schemaVersion")
                .and_then(serde_json::Value::as_u64),
            Some(u64::from(json::SCHEMA_VERSION))
        );
        assert!(value.get("data").is_some_and(serde_json::Value::is_array));
    }
}
