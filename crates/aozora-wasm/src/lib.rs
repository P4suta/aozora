//! WASM driver for the aozora parser.
//!
//! Compiles to a `wasm32-unknown-unknown` artifact suitable for
//! `wasm-pack build --target web`, exposing `aozora::Document` /
//! `aozora::AozoraTree` equivalents that JS / TypeScript consumers
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
//! Every JSON-returning method delegates to [`aozora::wire`], the
//! single authority for the cross-driver wire shape. `aozora-ffi` /
//! `aozora-wasm` / `aozora-py` emit byte-identical envelopes:
//!
//! ```json
//! { "schema_version": 1, "data": [ … ] }
//! ```
//!
//! [`aozora::wire::SCHEMA_VERSION`] bumps on any breaking change to
//! that shape.

#![forbid(unsafe_code)]

#[cfg(target_arch = "wasm32")]
mod bindings {
    use aozora::{Document as AozoraDoc, wire};
    use wasm_bindgen::prelude::*;

    /// JS-facing handle to a parsed Aozora document.
    ///
    /// Wraps an [`aozora::Document`] (which owns both the source and
    /// the bumpalo arena that backs the borrowed AST). Drop is
    /// automatic when the JS-side handle is GC'd.
    #[wasm_bindgen]
    pub struct Document {
        inner: AozoraDoc,
    }

    #[wasm_bindgen]
    impl Document {
        /// Construct from a UTF-16 JS string. The string is copied
        /// once into the Document's internal `Box<str>`; subsequent
        /// renders reuse the bumpalo arena.
        #[wasm_bindgen(constructor)]
        #[must_use]
        pub fn new(source: String) -> Self {
            Self {
                inner: AozoraDoc::new(source),
            }
        }

        /// Render the document to a semantic-HTML5 string.
        #[wasm_bindgen]
        #[must_use]
        pub fn to_html(&self) -> String {
            self.inner.parse().to_html()
        }

        /// Re-emit Aozora source text from the parse tree.
        #[wasm_bindgen]
        #[must_use]
        pub fn serialize(&self) -> String {
            self.inner.parse().serialize()
        }

        /// Diagnostics as JSON. Empty parse →
        /// `{"schema_version":1,"data":[]}`. Wire format defined in
        /// [`aozora::wire`].
        #[wasm_bindgen]
        #[must_use]
        pub fn diagnostics_json(&self) -> String {
            wire::serialize_diagnostics(self.inner.parse().diagnostics())
        }

        /// Source-keyed Aozora-node spans as JSON. Each entry is
        /// `{ kind, span: { start, end } }` where `kind` is the
        /// camelCase [`aozora::AozoraNode`] discriminant
        /// (`"ruby"` / `"bouten"` / `"gaiji"` / …) plus
        /// `"containerOpen"` / `"containerClose"` for container
        /// open / close markers. `span` covers source bytes, sorted
        /// by `span.start`.
        ///
        /// Stream-friendly for the aozora-obsidian Lezer-Tree builder
        /// — the underlying `source_nodes` table tiles spans
        /// contiguously by construction.
        #[wasm_bindgen]
        #[must_use]
        pub fn nodes_json(&self) -> String {
            wire::serialize_nodes(&self.inner.parse())
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
        #[wasm_bindgen]
        #[must_use]
        pub fn pairs_json(&self) -> String {
            wire::serialize_pairs(&self.inner.parse())
        }

        /// Source byte length. Useful for JS-side progress UI.
        #[wasm_bindgen]
        #[must_use]
        pub fn source_byte_len(&self) -> usize {
            self.inner.source().len()
        }
    }
}

#[cfg(test)]
mod tests {
    use aozora::{Document, wire};

    /// Diagnostics for plain input is the empty envelope.
    #[test]
    fn diagnostics_json_is_empty_envelope_for_clean_input() {
        let doc = Document::new("plain".to_owned());
        let json = wire::serialize_diagnostics(doc.parse().diagnostics());
        assert_eq!(json, r#"{"schema_version":1,"data":[]}"#);
    }

    /// PUA collision shows up as a `kind:"source_contains_pua"` entry
    /// inside the envelope.
    #[test]
    fn diagnostics_json_emits_pua_diagnostic() {
        let doc = Document::new("abc\u{E001}def".to_owned());
        let json = wire::serialize_diagnostics(doc.parse().diagnostics());
        assert!(
            json.contains(r#""kind":"source_contains_pua""#),
            "json missing diag kind: {json}"
        );
        assert!(
            json.contains(r#""schema_version":1"#),
            "json missing schema_version: {json}"
        );
    }

    /// Round-trip JSON parse: every wire output must be valid JSON
    /// that decodes to a `{ schema_version, data }` object.
    #[test]
    fn diagnostics_json_round_trips_envelope() {
        let doc = Document::new("abc\u{E001}def".to_owned());
        let json = wire::serialize_diagnostics(doc.parse().diagnostics());
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert!(parsed.is_object(), "wire root must be object");
        assert_eq!(
            parsed
                .get("schema_version")
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
        assert!(parsed.get("data").is_some_and(serde_json::Value::is_array));
    }

    /// Plain input has no Aozora-classified spans → empty envelope.
    #[test]
    fn nodes_json_is_empty_envelope_for_plain_text() {
        let doc = Document::new("hello, world".to_owned());
        let json = wire::serialize_nodes(&doc.parse());
        assert_eq!(json, r#"{"schema_version":1,"data":[]}"#);
    }

    /// Ruby span emits a `kind:"ruby"` entry.
    #[test]
    fn nodes_json_classifies_ruby() {
        let doc = Document::new("｜青梅《おうめ》".to_owned());
        let json = wire::serialize_nodes(&doc.parse());
        assert!(
            json.contains(r#""kind":"ruby""#),
            "json should mark ruby: {json}"
        );
    }

    /// Round-trip: every wire output is valid JSON with the expected
    /// envelope shape.
    #[test]
    fn nodes_json_round_trips_as_envelope() {
        let doc = Document::new("｜山《やま》や［＃改ページ］\n《《秘密》》".to_owned());
        let json = wire::serialize_nodes(&doc.parse());
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
        let json = wire::serialize_nodes(&doc.parse());
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
        let json = wire::serialize_pairs(&doc.parse());
        assert!(json.contains(r#""kind":"ruby""#), "pairs json: {json}");
        assert!(json.contains(r#""open":"#), "pairs json: {json}");
        assert!(json.contains(r#""close":"#), "pairs json: {json}");
    }
}
