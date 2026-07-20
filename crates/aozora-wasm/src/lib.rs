//! WASM driver for the aozora parser.
//!
//! Compiles to a `wasm32-unknown-unknown` artifact suitable for
//! `wasm-pack build --target web`, exposing `aozora::Document` /
//! `aozora::Snapshot` equivalents that JS / TypeScript consumers
//! can `import { Document } from "aozora-wasm"`.
//!
//! ## Build targeting
//!
//! The wasm-bindgen exports below are gated on
//! `cfg(target_arch = "wasm32")` so host builds of the cargo
//! workspace (`x86_64`, `aarch64`) skip them entirely. Add
//! `wasm32-unknown-unknown` via `rustup target add` before invoking
//! `wasm-pack build --target web --release crates/aozora-wasm`.
//!
//! When `aozora-scan` grows `wasm_simd` backend support, this crate's
//! release build picks it up via `-Ctarget-feature=+simd128`. The
//! size budget for the resulting `.wasm` artifact (post `wasm-opt
//! -O3 --enable-simd`) is ≤ 500 KiB.
//!
//! Structured methods return typed JavaScript values generated from the same
//! Rust wire records used by CLI and FFI JSON.

#![forbid(unsafe_code)]

/// Largest input the parser core accepts, in bytes. Its span offsets
/// are `u32`, so a source longer than this trips a `u32::MAX` assert
/// inside the lexer; under `panic = "abort"` that would tear down the
/// whole Wasm instance instead of surfacing a recoverable error.
///
/// Gated to `wasm32 || test`: the constant + its guard are consumed
/// only by the wasm-bindgen `Document` constructor and the host unit
/// test, so a plain host build (which compiles neither) would
/// otherwise see them as dead code under the workspace `dead_code =
/// "deny"`.
#[cfg(any(target_arch = "wasm32", test))]
const MAX_SOURCE_BYTES: usize = u32::MAX as usize;

/// Boundary check shared by the Wasm [`Document`] constructor.
///
/// Returns `Ok(())` when a source of `byte_len` UTF-8 bytes is within
/// the parser core's `u32` span-offset limit, or `Err` with a
/// human-readable message otherwise. Factored out of the
/// `cfg(target_arch = "wasm32")` bindings so the guard is unit-testable
/// on the host (where the wasm-bindgen surface is not compiled).
///
/// # Errors
///
/// `Err(&'static str)` when `byte_len > u32::MAX`.
#[cfg(any(target_arch = "wasm32", test))]
#[cfg_attr(
    target_arch = "wasm32",
    allow(
        clippy::absurd_extreme_comparisons,
        reason = "on wasm32 `usize` is `u32`, so `MAX_SOURCE_BYTES` (= u32::MAX) equals `usize::MAX` and this `> u32::MAX` guard is vacuously false there — a wasm32 `String` can never exceed the parser core's u32 span limit. The check is real and unit-tested on 64-bit hosts (usize = u64) under cfg(test)."
    )
)]
const fn source_len_within_span_limit(byte_len: usize) -> Result<(), &'static str> {
    if byte_len > MAX_SOURCE_BYTES {
        return Err("source exceeds 4 GiB (u32::MAX) span limit");
    }
    Ok(())
}

/// `wasm-bindgen` exports — the JavaScript-facing surface of the parser.
#[cfg(target_arch = "wasm32")]
pub mod bindings {
    use aozora::{Document as AozoraDoc, TextEdit, json};
    use serde::Deserialize;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(typescript_custom_section)]
    const GENERATED_TYPES: &str = include_str!("../types/aozora_types.d.ts");

    #[derive(Deserialize)]
    struct EditInput {
        start: usize,
        end: usize,
        replacement: String,
    }

    /// The completion catalogue as typed JavaScript values.
    ///
    /// # Errors
    ///
    /// Returns `Err(JsValue)` if the generated wire records cannot be
    /// converted to JavaScript values.
    #[wasm_bindgen(js_name = slugs, unchecked_return_type = "ReadonlyArray<Slug>")]
    pub fn slugs() -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(&json::slug_entries())
            .map_err(|err| JsValue::from_str(&err.to_string()))
    }

    /// Force one-time parser-table initialisation off the
    /// first-keystroke critical path.
    ///
    /// (SIMD backend choice + annotation-classifier DFA.) Idempotent. The
    /// playground calls this right after `init()` resolves — before the
    /// editor is created — so the first keystroke parse does not pay the
    /// DFA build.
    #[wasm_bindgen]
    pub fn prewarm() {
        aozora::prewarm();
    }

    /// The parser's channel-aware build version.
    ///
    /// The playground renders this in its footer so a deployed build is
    /// traceable back to a commit. Single authority: the
    /// `AOZORA_VERSION_STRING` this crate's `build.rs` injects — never a
    /// hard-coded literal.
    #[wasm_bindgen]
    #[must_use]
    pub fn version() -> String {
        env!("AOZORA_VERSION_STRING").to_owned()
    }

    /// The wire schema version every `*Json` envelope carries in its
    /// `schemaVersion` field.
    ///
    /// Distinct from [`version`], which is the build/channel string: this
    /// is the single-authority [`aozora::json::SCHEMA_VERSION`], bumped
    /// only on a breaking change to the `{ schemaVersion, data }` shape.
    /// A JS host can assert it against the version it was compiled
    /// against before trusting an envelope's fields.
    #[wasm_bindgen(js_name = schemaVersion)]
    #[must_use]
    pub fn schema_version() -> u32 {
        json::SCHEMA_VERSION
    }

    /// JS-facing handle to a parsed Aozora document.
    ///
    /// Wraps an [`aozora::Document`] and its immutable parsed snapshot.
    /// Drop is automatic when the JS-side handle is GC'd.
    #[wasm_bindgen]
    #[derive(Debug)]
    pub struct Document {
        inner: AozoraDoc,
    }

    #[wasm_bindgen]
    impl Document {
        /// Construct from a UTF-16 JS string and parse it once.
        ///
        /// # Errors
        ///
        /// Returns `Err(JsValue)` when the UTF-8 byte length of `source`
        /// exceeds [`u32::MAX`] (~4 GiB). The parser core asserts that
        /// bound (its span offsets are `u32`), and under `panic =
        /// "abort"` the assert would abort the Wasm instance rather than
        /// surface a recoverable error — so we reject up front here.
        /// Guarding at construction means no oversize `Document` exists,
        /// so the parse-driven methods below never hit the assert.
        #[wasm_bindgen(constructor)]
        pub fn new(source: String) -> Result<Self, JsValue> {
            crate::source_len_within_span_limit(source.len()).map_err(JsValue::from_str)?;
            Ok(Self {
                inner: aozora::parse(source).map_err(|err| JsValue::from_str(&err.to_string()))?,
            })
        }

        /// Render the document to a semantic-HTML5 string.
        #[wasm_bindgen(js_name = toHtml)]
        #[must_use]
        pub fn to_html(&self) -> String {
            self.inner.snapshot().to_html()
        }

        /// Re-emit Aozora source text from the parse tree.
        #[wasm_bindgen(js_name = toSource)]
        #[must_use]
        pub fn to_source(&self) -> String {
            self.inner.snapshot().to_source()
        }

        /// Apply a sorted, disjoint edit batch in pre-edit UTF-8 byte offsets.
        ///
        /// # Errors
        ///
        /// Returns `Err(JsValue)` for malformed input or an invalid edit batch.
        #[wasm_bindgen]
        pub fn edit(
            &mut self,
            #[wasm_bindgen(unchecked_param_type = "ReadonlyArray<TextEdit>")] edits: JsValue,
        ) -> Result<(), JsValue> {
            let edits: Vec<EditInput> = serde_wasm_bindgen::from_value(edits)
                .map_err(|err| JsValue::from_str(&err.to_string()))?;
            self.inner
                .edit(edits.into_iter().map(|edit| {
                    TextEdit::new(edit.start..edit.end, edit.replacement.into_boxed_str())
                }))
                .map_err(|err| JsValue::from_str(&err.to_string()))
        }

        /// Diagnostics as a plain-text report (`miette`-free): one block
        /// per diagnostic with its code, span, message, and the offending
        /// source slice. A clean parse → empty string. For the
        /// machine-readable view use [`Document::diagnostics`].
        #[wasm_bindgen(js_name = diagnosticsText)]
        #[must_use]
        pub fn diagnostics_text(&self) -> String {
            aozora::diagnostics_text(self.inner.source(), self.inner.snapshot().diagnostics())
        }

        /// Source byte length. Useful for JS-side progress UI.
        #[wasm_bindgen(js_name = sourceByteLen)]
        #[must_use]
        pub fn source_byte_len(&self) -> usize {
            self.inner.source().len()
        }

        /// Resolve the gaiji reference at a source byte offset.
        ///
        /// # Errors
        ///
        /// Returns `Err(JsValue)` if the typed record cannot be converted.
        #[wasm_bindgen(
            js_name = gaijiAt,
            unchecked_return_type = "GaijiResolution | undefined"
        )]
        pub fn gaiji_at(&self, byte_offset: usize) -> Result<JsValue, JsValue> {
            serde_wasm_bindgen::to_value(&json::gaiji_entry_at(&self.inner.snapshot(), byte_offset))
                .map_err(|err| JsValue::from_str(&err.to_string()))
        }

        /// Source-keyed Aozora nodes as parsed JS objects.
        ///
        /// # Errors
        ///
        /// Returns `Err(JsValue)` if `serde-wasm-bindgen` cannot convert the
        /// node records to a JS value — not expected for a well-formed parse.
        #[wasm_bindgen(
            js_name = nodes,
            unchecked_return_type = "ReadonlyArray<Node>"
        )]
        pub fn nodes(&self) -> Result<JsValue, JsValue> {
            serde_wasm_bindgen::to_value(&json::node_entries(&self.inner.snapshot()))
                .map_err(|e| JsValue::from_str(&e.to_string()))
        }

        /// Matched open/close pairs as parsed JS objects.
        ///
        /// # Errors
        ///
        /// Returns `Err(JsValue)` if `serde-wasm-bindgen` cannot convert the
        /// pair records to a JS value — not expected for a well-formed parse.
        #[wasm_bindgen(
            js_name = pairs,
            unchecked_return_type = "ReadonlyArray<Pair>"
        )]
        pub fn pairs(&self) -> Result<JsValue, JsValue> {
            serde_wasm_bindgen::to_value(&json::pair_entries(&self.inner.snapshot()))
                .map_err(|e| JsValue::from_str(&e.to_string()))
        }

        /// Container open/close pairs as parsed JS objects.
        ///
        /// # Errors
        ///
        /// Returns `Err(JsValue)` if `serde-wasm-bindgen` cannot convert the
        /// container-pair records to a JS value — not expected for a
        /// well-formed parse.
        #[wasm_bindgen(
            js_name = containerPairs,
            unchecked_return_type = "ReadonlyArray<ContainerPair>"
        )]
        pub fn container_pairs(&self) -> Result<JsValue, JsValue> {
            serde_wasm_bindgen::to_value(&json::container_pair_entries(&self.inner.snapshot()))
                .map_err(|e| JsValue::from_str(&e.to_string()))
        }

        /// Diagnostics as parsed JS objects.
        ///
        /// # Errors
        ///
        /// Returns `Err(JsValue)` if `serde-wasm-bindgen` cannot convert the
        /// diagnostic records to a JS value — not expected for a well-formed
        /// parse.
        #[wasm_bindgen(
            js_name = diagnostics,
            unchecked_return_type = "ReadonlyArray<Diagnostic>"
        )]
        pub fn diagnostics(&self) -> Result<JsValue, JsValue> {
            serde_wasm_bindgen::to_value(&json::diagnostic_entries(
                self.inner.snapshot().diagnostics(),
            ))
            .map_err(|e| JsValue::from_str(&e.to_string()))
        }

        /// Gaiji resolutions as parsed JS objects.
        ///
        /// # Errors
        ///
        /// Returns `Err(JsValue)` if `serde-wasm-bindgen` cannot convert the
        /// gaiji records to a JS value — not expected for a well-formed parse.
        #[wasm_bindgen(
            js_name = gaiji,
            unchecked_return_type = "ReadonlyArray<GaijiResolution>"
        )]
        pub fn gaiji(&self) -> Result<JsValue, JsValue> {
            serde_wasm_bindgen::to_value(&json::gaiji_entries(&self.inner.snapshot()))
                .map_err(|e| JsValue::from_str(&e.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use aozora::json;

    /// The boundary guard accepts in-range lengths (including the
    /// inclusive `u32::MAX` upper bound) and rejects anything larger.
    /// This mirrors the `u32::MAX` assert the parser core enforces at
    /// `tokenize_in`; the Wasm `Document` constructor calls this guard
    /// so an oversize source surfaces as `Err(JsValue)` rather than a
    /// `panic = "abort"` teardown.
    #[test]
    fn source_len_guard_matches_u32_span_boundary() {
        super::source_len_within_span_limit(0).expect("empty source is in range");
        super::source_len_within_span_limit(1024).expect("1 KiB source is in range");
        // Largest accepted length is exactly u32::MAX.
        super::source_len_within_span_limit(u32::MAX as usize)
            .expect("u32::MAX bytes is the inclusive upper bound");
        // One byte past the limit is rejected.
        let err = super::source_len_within_span_limit(u32::MAX as usize + 1)
            .expect_err("u32::MAX + 1 bytes must be rejected");
        assert!(err.contains("u32::MAX"), "error mentions the limit: {err}");
    }

    /// Diagnostics for plain input is the empty envelope.
    #[test]
    fn diagnostics_json_is_empty_envelope_for_clean_input() {
        let doc = aozora::parse("plain".to_owned()).expect("source fits parser span limit");
        let json = json::diagnostics(doc.snapshot().diagnostics());
        assert_eq!(
            json,
            format!(r#"{{"schemaVersion":{},"data":[]}}"#, json::SCHEMA_VERSION)
        );
    }

    /// PUA collision shows up as a `kind:"source_contains_pua"` entry
    /// inside the envelope.
    #[test]
    fn diagnostics_json_emits_pua_diagnostic() {
        let doc =
            aozora::parse("abc\u{E001}def".to_owned()).expect("source fits parser span limit");
        let json = json::diagnostics(doc.snapshot().diagnostics());
        assert!(
            json.contains(r#""kind":"source_contains_pua""#),
            "json missing diag kind: {json}"
        );
        assert!(
            json.contains(&format!(r#""schemaVersion":{}"#, json::SCHEMA_VERSION)),
            "json missing schemaVersion: {json}"
        );
    }

    /// Round-trip JSON parse: every wire output must be valid JSON
    /// that decodes to a `{ schemaVersion, data }` object.
    #[test]
    fn diagnostics_json_round_trips_envelope() {
        let doc =
            aozora::parse("abc\u{E001}def".to_owned()).expect("source fits parser span limit");
        let json = json::diagnostics(doc.snapshot().diagnostics());
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert!(parsed.is_object(), "wire root must be object");
        assert_eq!(
            parsed
                .get("schemaVersion")
                .and_then(serde_json::Value::as_u64),
            Some(u64::from(json::SCHEMA_VERSION))
        );
        assert!(parsed.get("data").is_some_and(serde_json::Value::is_array));
    }

    /// Plain input has no Aozora-classified spans → empty envelope.
    #[test]
    fn nodes_json_is_empty_envelope_for_plain_text() {
        let doc = aozora::parse("hello, world".to_owned()).expect("source fits parser span limit");
        let json = json::nodes(&doc.snapshot());
        assert_eq!(
            json,
            format!(r#"{{"schemaVersion":{},"data":[]}}"#, json::SCHEMA_VERSION)
        );
    }

    /// Ruby span emits a `kind:"ruby"` entry.
    #[test]
    fn nodes_json_classifies_ruby() {
        let doc =
            aozora::parse("｜青梅《おうめ》".to_owned()).expect("source fits parser span limit");
        let json = json::nodes(&doc.snapshot());
        assert!(
            json.contains(r#""kind":"ruby""#),
            "json should mark ruby: {json}"
        );
    }

    /// Round-trip: every wire output is valid JSON with the expected
    /// envelope shape.
    #[test]
    fn nodes_json_round_trips_as_envelope() {
        let doc = aozora::parse("｜山《やま》や［＃改ページ］\n≪秘密≫".to_owned())
            .expect("source fits parser span limit");
        let json = json::nodes(&doc.snapshot());
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let arr = parsed
            .get("data")
            .and_then(|v| v.as_array())
            .expect("data is array");
        assert!(!arr.is_empty(), "should have classified at least one node");
        for entry in arr {
            assert!(entry.get("kind").is_some(), "entry missing kind");
            let span = entry.get("span").expect("entry missing span");
            assert!(span.get("start").is_some(), "span missing start");
            assert!(span.get("end").is_some(), "span missing end");
        }
    }

    /// Source-order property: `data` entries are sorted by
    /// `span.start` ascending.
    #[test]
    fn nodes_json_spans_are_in_source_order() {
        let doc = aozora::parse("｜山《やま》。｜川《かわ》。｜空《そら》。".to_owned())
            .expect("source fits parser span limit");
        let json = json::nodes(&doc.snapshot());
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let arr = parsed
            .get("data")
            .and_then(|v| v.as_array())
            .expect("data is array");
        let starts: Vec<u64> = arr
            .iter()
            .filter_map(|v| {
                v.get("span")
                    .and_then(|s| s.get("start"))
                    .and_then(serde_json::Value::as_u64)
            })
            .collect();
        let mut sorted = starts.clone();
        sorted.sort_unstable();
        assert_eq!(starts, sorted, "spans must be emitted in source order");
    }

    /// Ruby pair appears in `pairs_json`.
    #[test]
    fn pairs_json_emits_ruby_pair() {
        let doc =
            aozora::parse("｜青梅《おうめ》".to_owned()).expect("source fits parser span limit");
        let json = json::pairs(&doc.snapshot());
        assert!(json.contains(r#""kind":"ruby""#), "pairs json: {json}");
        assert!(json.contains(r#""open":"#), "pairs json: {json}");
        assert!(json.contains(r#""close":"#), "pairs json: {json}");
    }
}
