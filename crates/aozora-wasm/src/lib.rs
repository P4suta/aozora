//! Exposes document parsing and editing to JavaScript through `wasm-bindgen` on `wasm32`.
//! Structured results use the same wire records as the other bindings.

#![forbid(unsafe_code)]

#[cfg(any(target_arch = "wasm32", test))]
const MAX_SOURCE_BYTES: usize = u32::MAX as usize;

/// Checks the parser's source byte-length limit.
///
/// # Errors
/// Returns an error when `byte_len` exceeds [`u32::MAX`].
#[cfg(any(target_arch = "wasm32", test))]
#[cfg_attr(
    target_arch = "wasm32",
    expect(
        clippy::absurd_extreme_comparisons,
        reason = "wasm32 source lengths cannot exceed the parser's u32 span limit"
    )
)]
const fn source_len_within_span_limit(byte_len: usize) -> Result<(), &'static str> {
    if byte_len > MAX_SOURCE_BYTES {
        return Err("source exceeds 4 GiB (u32::MAX) span limit");
    }
    Ok(())
}

/// JavaScript bindings for the parser.
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

    /// Returns the completion catalogue as JavaScript values.
    ///
    /// # Errors
    /// Returns an error if converting wire records to JavaScript fails.
    #[wasm_bindgen(js_name = slugs, unchecked_return_type = "ReadonlyArray<Slug>")]
    pub fn slugs() -> Result<JsValue, JsValue> {
        serde_wasm_bindgen::to_value(&json::slug_entries())
            .map_err(|err| JsValue::from_str(&err.to_string()))
    }

    /// Initializes parser tables before the first parse.
    /// Repeated calls have no effect.
    #[wasm_bindgen]
    pub fn prewarm() {
        aozora::prewarm();
    }

    /// Returns the parser's build version.
    #[wasm_bindgen]
    #[must_use]
    pub fn version() -> String {
        env!("AOZORA_VERSION_STRING").to_owned()
    }

    /// Returns the version of the structured wire format.
    #[wasm_bindgen(js_name = schemaVersion)]
    #[must_use]
    pub fn schema_version() -> u32 {
        json::SCHEMA_VERSION
    }

    /// A parsed Aozora document available to JavaScript.
    /// The generated JavaScript handle releases its allocation on garbage collection.
    #[wasm_bindgen]
    #[derive(Debug)]
    pub struct Document {
        inner: AozoraDoc,
    }

    #[wasm_bindgen]
    impl Document {
        /// Creates a document from a JavaScript string.
        ///
        /// # Errors
        /// Returns an error when the source's UTF-8 byte length exceeds [`u32::MAX`].
        #[wasm_bindgen(constructor)]
        pub fn new(source: String) -> Result<Self, JsValue> {
            crate::source_len_within_span_limit(source.len()).map_err(JsValue::from_str)?;
            Ok(Self {
                inner: aozora::parse(source).map_err(|err| JsValue::from_str(&err.to_string()))?,
            })
        }

        /// Renders the document as HTML.
        #[wasm_bindgen(js_name = toHtml)]
        #[must_use]
        pub fn to_html(&self) -> String {
            self.inner.snapshot().to_html()
        }

        /// Serializes the document as canonical Aozora source.
        #[wasm_bindgen(js_name = toSource)]
        #[must_use]
        pub fn to_source(&self) -> String {
            self.inner.snapshot().to_source()
        }

        /// Applies a sorted, disjoint edit batch using pre-edit UTF-8 byte offsets.
        ///
        /// # Errors
        /// Returns an error for malformed input or an invalid edit batch.
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

        /// Returns a plain-text diagnostic report, or an empty string for a clean parse.
        /// Use [`Document::diagnostics`] for structured records.
        #[wasm_bindgen(js_name = diagnosticsText)]
        #[must_use]
        pub fn diagnostics_text(&self) -> String {
            aozora::diagnostics_text(self.inner.source(), self.inner.snapshot().diagnostics())
        }

        /// Returns the source length in UTF-8 bytes.
        #[wasm_bindgen(js_name = sourceByteLen)]
        #[must_use]
        pub fn source_byte_len(&self) -> usize {
            self.inner.source().len()
        }

        /// Resolves the gaiji reference at a source byte offset.
        ///
        /// # Errors
        /// Returns an error if converting the wire record to JavaScript fails.
        #[wasm_bindgen(
            js_name = gaijiAt,
            unchecked_return_type = "GaijiResolution | undefined"
        )]
        pub fn gaiji_at(&self, byte_offset: usize) -> Result<JsValue, JsValue> {
            serde_wasm_bindgen::to_value(&json::gaiji_entry_at(&self.inner.snapshot(), byte_offset))
                .map_err(|err| JsValue::from_str(&err.to_string()))
        }

        /// Returns nodes as JavaScript objects with original-source byte spans.
        ///
        /// # Errors
        /// Returns an error if converting wire records to JavaScript fails.
        #[wasm_bindgen(
            js_name = nodes,
            unchecked_return_type = "ReadonlyArray<Node>"
        )]
        pub fn nodes(&self) -> Result<JsValue, JsValue> {
            serde_wasm_bindgen::to_value(&json::node_entries(&self.inner.snapshot()))
                .map_err(|e| JsValue::from_str(&e.to_string()))
        }

        /// Returns matched delimiters as JavaScript objects with original-source byte spans.
        ///
        /// # Errors
        /// Returns an error if converting wire records to JavaScript fails.
        #[wasm_bindgen(
            js_name = pairs,
            unchecked_return_type = "ReadonlyArray<Pair>"
        )]
        pub fn pairs(&self) -> Result<JsValue, JsValue> {
            serde_wasm_bindgen::to_value(&json::pair_entries(&self.inner.snapshot()))
                .map_err(|e| JsValue::from_str(&e.to_string()))
        }

        /// Returns matched container delimiters as JavaScript objects with original-source byte spans.
        ///
        /// # Errors
        /// Returns an error if converting wire records to JavaScript fails.
        #[wasm_bindgen(
            js_name = containerPairs,
            unchecked_return_type = "ReadonlyArray<ContainerPair>"
        )]
        pub fn container_pairs(&self) -> Result<JsValue, JsValue> {
            serde_wasm_bindgen::to_value(&json::container_pair_entries(&self.inner.snapshot()))
                .map_err(|e| JsValue::from_str(&e.to_string()))
        }

        /// Returns diagnostics as JavaScript objects.
        ///
        /// # Errors
        /// Returns an error if converting wire records to JavaScript fails.
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

        /// Returns gaiji resolutions as JavaScript objects.
        ///
        /// # Errors
        /// Returns an error if converting wire records to JavaScript fails.
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

    #[test]
    fn source_len_guard_matches_u32_span_boundary() {
        super::source_len_within_span_limit(0).expect("empty source is in range");
        super::source_len_within_span_limit(1024).expect("1 KiB source is in range");
        super::source_len_within_span_limit(u32::MAX as usize)
            .expect("u32::MAX bytes is the inclusive upper bound");
        let err = super::source_len_within_span_limit(u32::MAX as usize + 1)
            .expect_err("u32::MAX + 1 bytes must be rejected");
        assert!(err.contains("u32::MAX"), "error mentions the limit: {err}");
    }

    #[test]
    fn diagnostics_json_is_empty_envelope_for_clean_input() {
        let doc = aozora::parse("plain".to_owned()).expect("source fits parser span limit");
        let json = json::diagnostics(doc.snapshot().diagnostics());
        assert_eq!(
            json,
            format!(r#"{{"schemaVersion":{},"data":[]}}"#, json::SCHEMA_VERSION)
        );
    }

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

    #[test]
    fn nodes_json_is_empty_envelope_for_plain_text() {
        let doc = aozora::parse("hello, world".to_owned()).expect("source fits parser span limit");
        let json = json::nodes(&doc.snapshot());
        assert_eq!(
            json,
            format!(r#"{{"schemaVersion":{},"data":[]}}"#, json::SCHEMA_VERSION)
        );
    }

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
