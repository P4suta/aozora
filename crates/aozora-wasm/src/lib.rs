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

#![forbid(unsafe_code)]

#[cfg(target_arch = "wasm32")]
mod bindings {
    use wasm_bindgen::prelude::*;

    /// JS-facing handle to a parsed Aozora document.
    ///
    /// Wraps an [`aozora::Document`] (which owns both the source and
    /// the bumpalo arena that backs the borrowed AST). Drop is
    /// automatic when the JS-side handle is GC'd.
    #[wasm_bindgen]
    pub struct Document {
        inner: aozora::Document,
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
                inner: aozora::Document::new(source),
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

        /// Diagnostics as a JSON string. Empty parse → `"[]"`.
        #[wasm_bindgen]
        #[must_use]
        pub fn diagnostics_json(&self) -> String {
            crate::diagnostics_json_view(self.inner.parse().diagnostics())
        }

        /// Source byte length. Useful for JS-side progress UI.
        #[wasm_bindgen]
        #[must_use]
        pub fn source_byte_len(&self) -> usize {
            self.inner.source().len()
        }
    }
}

/// Diagnostic projection mirrored from `aozora-ffi` so both drivers
/// emit identical JSON shapes. Public so the WASM bindings can call
/// it; useful in unit tests too.
#[must_use]
pub fn diagnostics_json_view(diagnostics: &[aozora::Diagnostic]) -> String {
    #[derive(serde::Serialize)]
    struct DiagnosticView<'a> {
        kind: &'static str,
        span_start: u32,
        span_end: u32,
        codepoint: Option<char>,
        #[serde(skip_serializing_if = "Option::is_none")]
        _phantom: Option<&'a ()>,
    }

    let views: Vec<DiagnosticView<'_>> = diagnostics
        .iter()
        .map(|d| match d {
            aozora::Diagnostic::SourceContainsPua {
                codepoint, span, ..
            } => DiagnosticView {
                kind: "source_contains_pua",
                span_start: span.start,
                span_end: span.end,
                codepoint: Some(*codepoint),
                _phantom: None,
            },
            aozora::Diagnostic::UnclosedBracket { span, .. } => DiagnosticView {
                kind: "unclosed_bracket",
                span_start: span.start,
                span_end: span.end,
                codepoint: None,
                _phantom: None,
            },
            aozora::Diagnostic::UnmatchedClose { span, .. } => DiagnosticView {
                kind: "unmatched_close",
                span_start: span.start,
                span_end: span.end,
                codepoint: None,
                _phantom: None,
            },
            aozora::Diagnostic::ResidualAnnotationMarker { span, .. } => DiagnosticView {
                kind: "residual_annotation_marker",
                span_start: span.start,
                span_end: span.end,
                codepoint: None,
                _phantom: None,
            },
            aozora::Diagnostic::UnregisteredSentinel {
                codepoint, span, ..
            } => DiagnosticView {
                kind: "unregistered_sentinel",
                span_start: span.start,
                span_end: span.end,
                codepoint: Some(*codepoint),
                _phantom: None,
            },
            aozora::Diagnostic::RegistryOutOfOrder { span, .. } => DiagnosticView {
                kind: "registry_out_of_order",
                span_start: span.start,
                span_end: span.end,
                codepoint: None,
                _phantom: None,
            },
            aozora::Diagnostic::RegistryPositionMismatch { expected, span, .. } => DiagnosticView {
                kind: "registry_position_mismatch",
                span_start: span.start,
                span_end: span.end,
                codepoint: Some(*expected),
                _phantom: None,
            },
            _ => DiagnosticView {
                kind: "unknown",
                span_start: 0,
                span_end: 0,
                codepoint: None,
                _phantom: None,
            },
        })
        .collect();

    serde_json::to_string(&views).unwrap_or_else(|_| "[]".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_json_view_emits_empty_array_for_no_diagnostics() {
        let json = diagnostics_json_view(&[]);
        assert_eq!(json, "[]");
    }

    #[test]
    fn diagnostics_json_view_emits_pua_diagnostic() {
        let doc = aozora::Document::new("abc\u{E001}def".to_owned());
        let json = diagnostics_json_view(doc.parse().diagnostics());
        assert!(
            json.contains("source_contains_pua"),
            "json missing diag kind: {json}"
        );
    }

    #[test]
    fn diagnostics_json_view_is_valid_json() {
        let doc = aozora::Document::new("abc\u{E001}def".to_owned());
        let json = diagnostics_json_view(doc.parse().diagnostics());
        // Round-trip parse via serde_json — fails the test if the
        // produced string isn't valid JSON.
        let parsed_json: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert!(parsed_json.is_array(), "diagnostics JSON must be an array");
    }
}
