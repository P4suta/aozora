//! WASM driver for the aozora parser.
//!
//! Compiles to a `wasm32-unknown-unknown` artifact suitable for
//! `wasm-pack build --target web`, exposing `aozora::Document` /
//! `aozora::Tree` equivalents that JS / TypeScript consumers
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
//! ## Wire format
//!
//! Every JSON-returning method delegates to [`aozora::json`], the
//! single authority for the cross-driver wire shape. `aozora-ffi` /
//! `aozora-wasm` / `aozora-py` emit byte-identical envelopes:
//!
//! ```json
//! { "schemaVersion": 1, "data": [ … ] }
//! ```
//!
//! [`aozora::json::SCHEMA_VERSION`] bumps on any breaking change to
//! that shape.

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
/// Each function takes/returns JSON strings in the standard wire
/// envelope (`aozora::json`); only compiled for the `wasm32` target.
#[cfg(target_arch = "wasm32")]
pub mod bindings {
    use aozora::{Document as AozoraDoc, json};
    use wasm_bindgen::prelude::*;

    /// All canonical slugs from the spec, packaged in the standard
    /// wire envelope so JS completion menus can drive a
    /// `［＃...］` catalogue without re-implementing the table.
    ///
    /// Each `data[]` entry: `{ canonical, family, accepts_param, doc, partner }`.
    /// `family` is the camelCase form of the Rust enum variant. Projection
    /// is the single authority in [`aozora::json::slugs`].
    #[wasm_bindgen(js_name = slugsJson)]
    #[must_use]
    pub fn slugs_json() -> String {
        json::slugs()
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

    /// High-resolution wall-clock for the profile helper. Returns
    /// milliseconds (f64) using the browser `performance.now()` so
    /// sub-millisecond precision is preserved. Falls back to 0.0 if
    /// the host has no `Performance` interface (e.g. a Worker that
    /// hasn't been told to expose one) — the deltas come out as
    /// constant 0 in that case, which is the least-surprising
    /// fallback for the profile UI.
    fn now_ms() -> f64 {
        web_sys::window()
            .and_then(|w| w.performance())
            .map_or(0.0, |p| p.now())
    }

    /// JS-facing handle to a parsed Aozora document.
    ///
    /// Wraps an [`aozora::Document`] (which owns both the source and
    /// the bumpalo arena that backs the borrowed AST). Drop is
    /// automatic when the JS-side handle is GC'd.
    #[wasm_bindgen]
    #[derive(Debug)]
    pub struct Document {
        inner: AozoraDoc,
    }

    #[wasm_bindgen]
    impl Document {
        /// Construct from a UTF-16 JS string. The string is copied
        /// once into the Document's internal `Box<str>`; subsequent
        /// renders reuse the bumpalo arena.
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
                inner: AozoraDoc::new(source),
            })
        }

        /// Render the document to a semantic-HTML5 string.
        #[wasm_bindgen(js_name = toHtml)]
        #[must_use]
        pub fn to_html(&self) -> String {
            self.inner.parse().to_html()
        }

        /// Re-emit Aozora source text from the parse tree.
        #[wasm_bindgen(js_name = toSource)]
        #[must_use]
        pub fn serialize(&self) -> String {
            self.inner.parse().to_source()
        }

        /// Diagnostics as JSON. Empty parse →
        /// `{"schemaVersion":1,"data":[]}`. Wire format defined in
        /// [`aozora::json`].
        #[wasm_bindgen(js_name = diagnosticsJson)]
        #[must_use]
        pub fn diagnostics_json(&self) -> String {
            json::diagnostics(self.inner.parse().diagnostics())
        }

        /// Diagnostics as a plain-text report (`miette`-free): one block
        /// per diagnostic with its code, span, message, and the offending
        /// source slice. A clean parse → empty string. For the
        /// machine-readable view use `diagnosticsJson`.
        #[wasm_bindgen(js_name = diagnosticsText)]
        #[must_use]
        pub fn diagnostics_text(&self) -> String {
            aozora::diagnostics_text(self.inner.source(), self.inner.parse().diagnostics())
        }

        /// Source-keyed Aozora-node spans as JSON. Each entry is
        /// `{ kind, span: { start, end } }` where `kind` is the
        /// camelCase [`aozora::Node`] discriminant
        /// (`"ruby"` / `"bouten"` / `"gaiji"` / …) plus
        /// `"containerOpen"` / `"containerClose"` for container
        /// open / close markers. `span` covers source bytes, sorted
        /// by `span.start`.
        ///
        /// Stream-friendly for the aozora-obsidian Lezer-Tree builder
        /// — the underlying `source_nodes` table tiles spans
        /// contiguously by construction.
        #[wasm_bindgen(js_name = nodesJson)]
        #[must_use]
        pub fn nodes_json(&self) -> String {
            json::nodes(&self.inner.parse())
        }

        /// Matched open/close pair links as JSON. Each entry is
        /// `{ kind, open: { start, end }, close: { start, end } }` in
        /// sanitized-source coordinates. Useful for LSP requests like
        /// `textDocument/linkedEditingRange` and
        /// `textDocument/documentHighlight`.
        ///
        /// Unmatched closes and unclosed opens are excluded — they
        /// have no partner span and would only confuse editor
        /// surfaces.
        #[wasm_bindgen(js_name = pairsJson)]
        #[must_use]
        pub fn pairs_json(&self) -> String {
            json::pairs(&self.inner.parse())
        }

        /// Source byte length. Useful for JS-side progress UI.
        #[wasm_bindgen(js_name = sourceByteLen)]
        #[must_use]
        pub fn source_byte_len(&self) -> usize {
            self.inner.source().len()
        }

        /// Per-method timing snapshot for the current source.
        ///
        /// Each entry is `{ "name": string, "durationMs": number }`.
        /// Timings are taken via `performance.now()` on the host
        /// (`Instant::now()` panics on `wasm32-unknown-unknown`).
        ///
        /// `parse` is the cost of `Document::parse()` alone
        /// (constructing the borrowed AST). The render entries
        /// (`to_html`, `serialize`, `*_json`) are wall-clock for
        /// that single method call against the already-built tree —
        /// so summing them is the cost of "produce every output JS
        /// might want", and `parse` shows how much of the work
        /// happens up-front in the parser core itself.
        #[wasm_bindgen(js_name = profileJson)]
        #[must_use]
        pub fn profile_json(&self) -> String {
            let p0 = now_ms();
            let tree = self.inner.parse();
            let p1 = now_ms();

            let h0 = now_ms();
            let _html = tree.to_html();
            let h1 = now_ms();

            let s0 = now_ms();
            let _serialized = tree.to_source();
            let s1 = now_ms();

            let d0 = now_ms();
            let _diag = json::diagnostics(tree.diagnostics());
            let d1 = now_ms();

            let n0 = now_ms();
            let _nodes = json::nodes(&tree);
            let n1 = now_ms();

            let pa0 = now_ms();
            let _pairs = json::pairs(&tree);
            let pa1 = now_ms();

            // gaiji_resolutions_json scans the source string directly
            // (not the parse tree), so we time it separately by
            // invoking the same code path the JS-side method uses.
            let g0 = now_ms();
            let _gaiji = self.gaiji_resolutions_json();
            let g1 = now_ms();

            let entries = serde_json::json!([
                { "name": "parse",             "durationMs": p1  - p0  },
                { "name": "to_html",           "durationMs": h1  - h0  },
                { "name": "serialize",         "durationMs": s1  - s0  },
                { "name": "diagnostics_json",  "durationMs": d1  - d0  },
                { "name": "nodes_json",        "durationMs": n1  - n0  },
                { "name": "pairs_json",        "durationMs": pa1 - pa0 },
                { "name": "gaiji_resolutions", "durationMs": g1  - g0  },
            ]);
            serde_json::json!({
                "schemaVersion": 1,
                "byteLen": self.inner.source().len(),
                "data": entries,
            })
            .to_string()
        }

        /// Resolve the gaiji reference at `byte_offset` (if any) and
        /// return a JSON object describing it. Returns the literal
        /// string `"null"` if the offset is not inside a gaiji span
        /// or the body cannot be parsed.
        ///
        /// JSON shape on hit:
        /// ```json
        /// { "span": { "start": u, "end": u },
        ///   "description": string,
        ///   "mencode": string | null,
        ///   "codepoint": u32 | null,
        ///   "resolved": string | null }
        /// ```
        ///
        /// Locality: the scan is bounded to a 512-byte window either
        /// side of `byte_offset`, so the cost is independent of
        /// document size. Editors call this on every cursor move.
        #[wasm_bindgen(js_name = resolveGaijiAt)]
        #[must_use]
        pub fn resolve_gaiji_at(&self, byte_offset: usize) -> String {
            json::gaiji_at(self.inner.source(), byte_offset)
        }

        /// All gaiji resolutions found in the document, packaged in
        /// the standard `{schema_version, data:[...]}` wire envelope.
        /// Powers inlay-hint UIs that show `→GLYPH` after every
        /// `※［＃…］` span.
        ///
        /// Walks the source linearly once; cost is `O(source)`.
        #[wasm_bindgen(js_name = gaijiJson)]
        #[must_use]
        pub fn gaiji_resolutions_json(&self) -> String {
            json::gaiji(self.inner.source())
        }

        /// Container open/close pairs as a raw wire-envelope string.
        #[wasm_bindgen(js_name = containerPairsJson)]
        #[must_use]
        pub fn container_pairs_json(&self) -> String {
            json::container_pairs(&self.inner.parse())
        }

        // ── Structured accessors ──────────────────────────────────
        // First-class parsed `data[]` (JS objects). The `*Json` methods
        // above remain the raw-string escape hatch.

        /// Source-keyed Aozora nodes as parsed JS objects.
        ///
        /// # Errors
        ///
        /// Returns `Err(JsValue)` if `serde-wasm-bindgen` cannot convert the
        /// node records to a JS value — not expected for a well-formed parse.
        #[wasm_bindgen(js_name = nodes)]
        pub fn nodes(&self) -> Result<JsValue, JsValue> {
            serde_wasm_bindgen::to_value(&json::node_entries(&self.inner.parse()))
                .map_err(|e| JsValue::from_str(&e.to_string()))
        }

        /// Matched open/close pairs as parsed JS objects.
        ///
        /// # Errors
        ///
        /// Returns `Err(JsValue)` if `serde-wasm-bindgen` cannot convert the
        /// pair records to a JS value — not expected for a well-formed parse.
        #[wasm_bindgen(js_name = pairs)]
        pub fn pairs(&self) -> Result<JsValue, JsValue> {
            serde_wasm_bindgen::to_value(&json::pair_entries(&self.inner.parse()))
                .map_err(|e| JsValue::from_str(&e.to_string()))
        }

        /// Container open/close pairs as parsed JS objects.
        ///
        /// # Errors
        ///
        /// Returns `Err(JsValue)` if `serde-wasm-bindgen` cannot convert the
        /// container-pair records to a JS value — not expected for a
        /// well-formed parse.
        #[wasm_bindgen(js_name = containerPairs)]
        pub fn container_pairs(&self) -> Result<JsValue, JsValue> {
            serde_wasm_bindgen::to_value(&json::container_pair_entries(&self.inner.parse()))
                .map_err(|e| JsValue::from_str(&e.to_string()))
        }

        /// Diagnostics as parsed JS objects.
        ///
        /// # Errors
        ///
        /// Returns `Err(JsValue)` if `serde-wasm-bindgen` cannot convert the
        /// diagnostic records to a JS value — not expected for a well-formed
        /// parse.
        #[wasm_bindgen(js_name = diagnostics)]
        pub fn diagnostics(&self) -> Result<JsValue, JsValue> {
            serde_wasm_bindgen::to_value(&json::diagnostic_entries(
                self.inner.parse().diagnostics(),
            ))
            .map_err(|e| JsValue::from_str(&e.to_string()))
        }

        /// Gaiji resolutions as parsed JS objects.
        ///
        /// # Errors
        ///
        /// Returns `Err(JsValue)` if `serde-wasm-bindgen` cannot convert the
        /// gaiji records to a JS value — not expected for a well-formed parse.
        #[wasm_bindgen(js_name = gaiji)]
        pub fn gaiji(&self) -> Result<JsValue, JsValue> {
            serde_wasm_bindgen::to_value(&json::gaiji_entries(self.inner.source()))
                .map_err(|e| JsValue::from_str(&e.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use aozora::{Document, json};

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
        let doc = Document::new("plain".to_owned());
        let json = json::diagnostics(doc.parse().diagnostics());
        assert_eq!(json, r#"{"schemaVersion":1,"data":[]}"#);
    }

    /// PUA collision shows up as a `kind:"source_contains_pua"` entry
    /// inside the envelope.
    #[test]
    fn diagnostics_json_emits_pua_diagnostic() {
        let doc = Document::new("abc\u{E001}def".to_owned());
        let json = json::diagnostics(doc.parse().diagnostics());
        assert!(
            json.contains(r#""kind":"source_contains_pua""#),
            "json missing diag kind: {json}"
        );
        assert!(
            json.contains(r#""schemaVersion":1"#),
            "json missing schema_version: {json}"
        );
    }

    /// Round-trip JSON parse: every wire output must be valid JSON
    /// that decodes to a `{ schema_version, data }` object.
    #[test]
    fn diagnostics_json_round_trips_envelope() {
        let doc = Document::new("abc\u{E001}def".to_owned());
        let json = json::diagnostics(doc.parse().diagnostics());
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert!(parsed.is_object(), "wire root must be object");
        assert_eq!(
            parsed
                .get("schemaVersion")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert!(parsed.get("data").is_some_and(serde_json::Value::is_array));
    }

    /// Plain input has no Aozora-classified spans → empty envelope.
    #[test]
    fn nodes_json_is_empty_envelope_for_plain_text() {
        let doc = Document::new("hello, world".to_owned());
        let json = json::nodes(&doc.parse());
        assert_eq!(json, r#"{"schemaVersion":1,"data":[]}"#);
    }

    /// Ruby span emits a `kind:"ruby"` entry.
    #[test]
    fn nodes_json_classifies_ruby() {
        let doc = Document::new("｜青梅《おうめ》".to_owned());
        let json = json::nodes(&doc.parse());
        assert!(
            json.contains(r#""kind":"ruby""#),
            "json should mark ruby: {json}"
        );
    }

    /// Round-trip: every wire output is valid JSON with the expected
    /// envelope shape.
    #[test]
    fn nodes_json_round_trips_as_envelope() {
        let doc = Document::new("｜山《やま》や［＃改ページ］\n≪秘密≫".to_owned());
        let json = json::nodes(&doc.parse());
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
        let doc = Document::new("｜山《やま》。｜川《かわ》。｜空《そら》。".to_owned());
        let json = json::nodes(&doc.parse());
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
        let doc = Document::new("｜青梅《おうめ》".to_owned());
        let json = json::pairs(&doc.parse());
        assert!(json.contains(r#""kind":"ruby""#), "pairs json: {json}");
        assert!(json.contains(r#""open":"#), "pairs json: {json}");
        assert!(json.contains(r#""close":"#), "pairs json: {json}");
    }
}
