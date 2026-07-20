//! JSON-format round-trip invariants.
//!
//! `tests/json_format.rs` pins the JSON output for a fixed family of
//! 23 hand-curated fixtures (severity / source axis / per-variant tag
//! shape). That gives a strong contract for *known* shapes but says
//! nothing about Aozora source the fixture set hasn't enumerated.
//!
//! The properties below project the four JSON envelopes
//! ([`diagnostics`], [`nodes`], [`pairs`],
//! [`container_pairs`]) onto *arbitrary* Aozora input drawn
//! from the workhorse generators, and assert two cross-cutting
//! invariants on each:
//!
//! 1. **Always parses as JSON.** Whatever the source, the JSON output
//!    is a syntactically valid JSON document. A regression that emits
//!    an unescaped control byte or a trailing comma corrupts every
//!    driver (`aozora-ffi` / `aozora-wasm` / `aozora-py`) at once and
//!    is caught here under shrinking.
//! 2. **Round-trip is canonical.** Parsing the JSON output as
//!    `serde_json::Value` and re-serialising it via `serde_json::to_string`
//!    yields a byte-identical envelope (we use the JSON projection's own
//!    canonical-key ordering by re-projecting through `Value`'s
//!    `Serialize` impl, which preserves the source object key order).
//!    A regression that introduces non-canonical key ordering breaks
//!    cross-driver byte parity even when each driver still parses
//!    valid JSON.
//!
//! Together these close the gap between "fixture-checked JSON shape"
//! and "JSON output stable across the entire input space".

#![cfg(feature = "json")]

use aozora::json;
use aozora_proptest::config::default_config;
use aozora_proptest::generators::*;
use proptest::prelude::*;

fn assert_envelope_is_well_formed_json(label: &str, source: &str, json: &str) {
    let parsed: serde_json::Value = serde_json::from_str(json).unwrap_or_else(|e| {
        panic!("{label} envelope is not valid JSON for source {source:?}\n{e}\n---\n{json}")
    });
    // Envelope shape contract: top-level object with `schemaVersion`
    // = 1 and a `data` array. A regression that drops either is a
    // JSON-shape break.
    let obj = parsed.as_object().unwrap_or_else(|| {
        panic!("{label} envelope must be a JSON object for source {source:?}\n---\n{json}")
    });
    let version = obj
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_else(|| {
            panic!("{label} envelope missing schemaVersion for source {source:?}\n---\n{json}")
        });
    assert_eq!(
        u32::try_from(version).ok(),
        Some(json::SCHEMA_VERSION),
        "{label} envelope schemaVersion drift for source {source:?}: got {version}, expected {expected}",
        expected = json::SCHEMA_VERSION,
    );
    assert!(
        obj.get("data").is_some_and(serde_json::Value::is_array),
        "{label} envelope missing data array for source {source:?}\n---\n{json}"
    );
}

fn assert_envelope_round_trips(label: &str, source: &str, json: &str) {
    let parsed: serde_json::Value =
        serde_json::from_str(json).expect("envelope already validated by caller");
    let reserialised = serde_json::to_string(&parsed).unwrap_or_else(|e| {
        panic!("{label} envelope refused to re-serialise for source {source:?}\n{e}")
    });
    // `serde_json::Value` preserves insertion order on its `Map`
    // backing store, so a JSON output whose key order is canonical
    // round-trips byte-equal. A regression that introduces dynamic
    // key ordering (e.g. a `HashMap` variant) breaks this.
    assert_eq!(
        json, reserialised,
        "{label} envelope round-trip is not byte-canonical for source {source:?}\n\
         original: {json}\nre-serialised: {reserialised}"
    );
}

fn assert_json_round_trip(source: &str) {
    let doc = aozora::parse(source).expect("source fits parser span limit");
    let tree = doc.snapshot();

    // (1) and (2) for each of the four envelope flavours.
    let diags = json::diagnostics(tree.diagnostics());
    assert_envelope_is_well_formed_json("diagnostics", source, &diags);
    assert_envelope_round_trips("diagnostics", source, &diags);

    let nodes = json::nodes(&tree);
    assert_envelope_is_well_formed_json("nodes", source, &nodes);
    assert_envelope_round_trips("nodes", source, &nodes);

    let pairs = json::pairs(&tree);
    assert_envelope_is_well_formed_json("pairs", source, &pairs);
    assert_envelope_round_trips("pairs", source, &pairs);

    let containers = json::container_pairs(&tree);
    assert_envelope_is_well_formed_json("container_pairs", source, &containers);
    assert_envelope_round_trips("container_pairs", source, &containers);
}

// ----------------------------------------------------------------------
// Hand-curated regression anchors.
// ----------------------------------------------------------------------

#[test]
fn empty_input_round_trips() {
    assert_json_round_trip("");
}

#[test]
fn plain_text_round_trips() {
    assert_json_round_trip("Hello, world.");
    assert_json_round_trip("こんにちは。\n\n本日は晴れ。");
}

#[test]
fn ruby_round_trips() {
    assert_json_round_trip("｜青梅《おうめ》");
    assert_json_round_trip("青梅《おうめ》");
}

#[test]
fn paired_container_round_trips() {
    assert_json_round_trip(
        "［＃ここから2字下げ］\n\
         body\n\
         ［＃ここで字下げ終わり］",
    );
}

#[test]
fn diagnostic_carrying_input_round_trips() {
    // Source that triggers a SourceContainsPua diagnostic — exercises
    // the diagnostics envelope on a non-empty data slice.
    assert_json_round_trip("a\u{E001}b");
}

proptest! {
    #![proptest_config(default_config())]

    /// Workhorse — every Aozora-shaped fragment must produce four
    /// envelopes that are valid JSON and byte-canonical after a
    /// `serde_json::Value` round-trip.
    #[test]
    fn aozora_fragment_json_round_trips(s in aozora_fragment(120)) {
        assert_json_round_trip(&s);
    }

    /// Pathological — unbalanced bracket shapes drive the diagnostics
    /// envelope hardest. Every diagnostic emitted must serialise into
    /// well-formed JSON regardless of how many fire.
    #[test]
    fn pathological_input_json_round_trips(s in pathological_aozora(120)) {
        assert_json_round_trip(&s);
    }

    /// Unicode adversarial — combining marks, RTL overrides, PUA
    /// codepoints. PUA inputs especially exercise the diagnostics
    /// envelope's `codepoint` field (which must JSON-escape the
    /// payload).
    #[test]
    fn unicode_adversarial_json_round_trips(s in unicode_adversarial()) {
        assert_json_round_trip(&s);
    }
}
