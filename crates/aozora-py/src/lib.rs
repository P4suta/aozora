//! Python (`PyO3`) driver for the aozora parser.
//!
//! Distributed via [`maturin`](https://www.maturin.rs/) as an
//! **abi3** wheel (`CPython` stable ABI, 3.11 floor): one
//! `cp311-abi3` wheel per platform loads on 3.11 … 3.14 and every
//! future 3.x. The `PyO3` binding surface is gated behind the
//! `extension-module` cargo feature so a plain `cargo build
//! --workspace` (without the feature) still succeeds; with
//! `abi3-py311` enabled, pyo3 also builds without discovering a
//! Python interpreter, so `cargo check -p aozora-py -F
//! extension-module` works inside the Python-less dev image.
//!
//! Build the wheel with either:
//!
//! - `maturin develop -F extension-module` from inside a Python venv, or
//! - `maturin build --release -F extension-module`.
//!
//! ## Layout
//!
//! The wheel ships a **mixed layout** (see `pyproject.toml`
//! `python-source`): the public, pure-Python package `aozora`
//! (`python/aozora/__init__.py`) wraps the compiled extension,
//! which is nested as the PRIVATE submodule `aozora._aozora`
//! built from this crate. The Python layer adds the idiomatic
//! surface — `Document.diagnostics()` / `nodes()` / `pairs()` /
//! `container_pairs()` return parsed `list[dict]`, while the raw,
//! byte-identical JSON stays available via the `*_json()` accessors
//! defined here.
//!
//! ## Wire format
//!
//! Every `*_json()` accessor delegates to [`aozora::json`], the
//! single authority for the cross-driver wire shape. `aozora-ffi` /
//! `aozora-wasm` / `aozora-py` emit byte-identical envelopes:
//!
//! ```json
//! { "schemaVersion": 1, "data": [ … ] }
//! ```

#![forbid(unsafe_code)]

#[cfg(feature = "extension-module")]
#[allow(
    clippy::too_many_arguments,
    reason = "the #[pyfunction] / #[pymethods] macros expand each fn into a Python ABI wrapper that PyO3 fills with extra context args (Python token, args, kwargs, …). The warning fires on the macro-generated signature, not on user code; per-item allow doesn't reach inside the macro expansion."
)]
mod bindings {
    use aozora::{Document as AozoraDoc, encoding, json};
    use pyo3::exceptions::PyValueError;
    use pyo3::prelude::*;

    /// Largest input the parser core accepts, in bytes. Span offsets
    /// are `u32`, so a longer source trips a `u32::MAX` assert inside
    /// the lexer; under `panic = "abort"` that tears down the whole
    /// interpreter instead of surfacing a recoverable error. The
    /// fallible entry points below reject oversize input up front so
    /// no oversize `Document` is ever constructed.
    const MAX_SOURCE_BYTES: usize = u32::MAX as usize;

    /// `PyO3`-facing handle to a parsed Aozora document.
    ///
    /// Wraps an [`AozoraDoc`], which owns only the source buffer
    /// (`Box<str>`) plus a `Copy` diagnostic policy and derives an owned,
    /// lifetime-free parse tree on demand. There is no arena and no
    /// interior-mutable state, so the wrapped document is itself
    /// `Send + Sync`.
    ///
    /// The `unsendable` marker below is therefore now *conservative*: it
    /// pins the `PyO3` handle to its constructing Python thread (access
    /// from another thread raises a `RuntimeError` rather than sharing
    /// it). That is a safe, stricter-than-required contract that could be
    /// relaxed now that the arena is gone — relaxing the attribute is a
    /// code change tracked separately.
    #[pyclass(unsendable)]
    #[derive(Debug)]
    pub struct Document {
        inner: AozoraDoc,
    }

    #[pymethods]
    impl Document {
        /// Construct from a Python `str`.
        #[new]
        fn new(source: &str) -> Self {
            Self {
                inner: AozoraDoc::new(source.to_owned()),
            }
        }

        /// Alternate constructor from raw bytes, decoding the source
        /// encoding automatically.
        ///
        /// Real 青空文庫 archive files are `Shift_JIS`; pre-converted
        /// corpora are UTF-8. [`encoding::decode_auto`] sniffs the two
        /// (valid UTF-8 wins, else `Shift_JIS`), so this accepts both.
        /// Despite the `from_bytes` name it is encoding-agnostic — the
        /// name reflects the common case (archive files).
        ///
        /// # Errors
        ///
        /// Raises `ValueError` if the bytes are neither valid UTF-8
        /// nor valid `Shift_JIS`, or if the decoded text exceeds the
        /// 4 GiB (`u32::MAX`) span limit.
        #[staticmethod]
        fn from_bytes(data: &[u8]) -> PyResult<Self> {
            let text = encoding::decode_auto(data)
                .map_err(|err| PyValueError::new_err(err.to_string()))?;
            if text.len() > MAX_SOURCE_BYTES {
                return Err(PyValueError::new_err(
                    "source exceeds 4 GiB (u32::MAX) span limit",
                ));
            }
            Ok(Self {
                inner: AozoraDoc::new(text.into_owned()),
            })
        }

        /// The source text the document was parsed from.
        #[getter]
        fn source(&self) -> &str {
            self.inner.source()
        }

        /// Render the document to HTML and return as a Python `str`.
        fn to_html(&self) -> String {
            self.inner.parse().to_html()
        }

        /// Re-emit Aozora source text.
        fn to_source(&self) -> String {
            self.inner.parse().to_source()
        }

        /// Diagnostics as a JSON envelope string. Empty parse →
        /// `{"schemaVersion":1,"data":[]}`. Wire format defined in
        /// [`aozora::json`]. The Python wrapper's `diagnostics()`
        /// returns the parsed `data` list; this is the raw,
        /// byte-identical accessor.
        fn diagnostics_json(&self) -> String {
            json::diagnostics(self.inner.parse().diagnostics())
        }

        /// Diagnostics as a plain-text report (`miette`-free): one block
        /// per diagnostic with its code, span, message, and the
        /// offending source slice. A clean parse → empty string. For the
        /// machine-readable view use `diagnostics_json`.
        fn diagnostics_text(&self) -> String {
            aozora::diagnostics_text(self.inner.source(), self.inner.parse().diagnostics())
        }

        /// Source-keyed Aozora-node spans as a JSON envelope string.
        /// See [`aozora::json::nodes`] for the schema.
        fn nodes_json(&self) -> String {
            json::nodes(&self.inner.parse())
        }

        /// Matched open/close pair links as a JSON envelope string.
        /// See [`aozora::json::pairs`] for the schema.
        fn pairs_json(&self) -> String {
            json::pairs(&self.inner.parse())
        }

        /// Container open/close pairs (indent / warichu / keigakomi /
        /// alignEnd / …) as a JSON envelope string, in normalized
        /// coordinates. See [`aozora::json::container_pairs`].
        /// Brings the Python surface to parity with the Go / Extism
        /// drivers.
        fn container_pairs_json(&self) -> String {
            json::container_pairs(&self.inner.parse())
        }

        /// Resolved gaiji references (`※［＃…］`) as a JSON envelope
        /// string. Each entry is `{ span: { start, end }, description,
        /// mencode, codepoint, resolved }` in source-byte coordinates.
        /// Scans the raw source (no parse needed). See
        /// [`aozora::json::gaiji`].
        fn gaiji_json(&self) -> String {
            json::gaiji(self.inner.source())
        }

        /// Source byte length.
        fn source_byte_len(&self) -> usize {
            self.inner.source().len()
        }
    }

    /// Module entry point — registered as `aozora._aozora` in
    /// Python (the private compiled submodule the pure-Python
    /// `aozora` package wraps). The function name must match the
    /// cdylib's `[lib] name` override in Cargo.toml.
    #[pymodule]
    fn _aozora(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add_class::<Document>()?;
        m.add_function(wrap_pyfunction!(parse_to_html, m)?)?;
        m.add_function(wrap_pyfunction!(prewarm, m)?)?;
        m.add_function(wrap_pyfunction!(decode_sjis, m)?)?;
        m.add_function(wrap_pyfunction!(slugs_json, m)?)?;
        Ok(())
    }

    /// Convenience: parse + render in one call.
    ///
    /// # Errors
    ///
    /// Raises `ValueError` if `source` exceeds the 4 GiB
    /// (`u32::MAX`) span limit.
    #[pyfunction]
    fn parse_to_html(source: &str) -> PyResult<String> {
        if source.len() > MAX_SOURCE_BYTES {
            return Err(PyValueError::new_err(
                "source exceeds 4 GiB (u32::MAX) span limit",
            ));
        }
        Ok(Document::new(source).to_html())
    }

    /// Force one-time parser-table initialisation (SIMD backend
    /// choice + annotation-classifier DFA) off the first-parse
    /// critical path. Idempotent. Batch workloads that parse many
    /// short documents call this once at startup so the first parse
    /// does not pay the DFA build.
    #[pyfunction]
    fn prewarm() {
        aozora::prewarm();
    }

    /// Decode `Shift_JIS` bytes to a Python `str`.
    ///
    /// Strict: no lossy replacement — callers need to know when they
    /// are looking at corrupted source rather than silently absorbing
    /// the damage. For encoding-agnostic decode (UTF-8 mirrors too),
    /// construct via [`Document::from_bytes`] instead.
    ///
    /// # Errors
    ///
    /// Raises `ValueError` on a malformed `Shift_JIS` byte sequence.
    #[pyfunction]
    fn decode_sjis(data: &[u8]) -> PyResult<String> {
        encoding::decode_sjis(data).map_err(|err| PyValueError::new_err(err.to_string()))
    }

    /// Canonical slug catalogue (`［＃…］` annotations) as a JSON
    /// envelope string. Static — independent of any document; useful
    /// for driving editor completion menus. See
    /// [`aozora::json::slugs`].
    #[pyfunction]
    fn slugs_json() -> String {
        json::slugs()
    }
}

#[cfg(test)]
mod tests {
    use aozora::{Document as AozoraDoc, json};

    /// Smoke: PUA collision shows up via `aozora::json`.
    #[test]
    fn diagnostics_through_wire_emits_pua_kind() {
        let doc = AozoraDoc::new("abc\u{E001}def".to_owned());
        let json = json::diagnostics(doc.parse().diagnostics());
        assert!(json.contains("source_contains_pua"), "json: {json}");
    }

    /// Smoke: clean parse → empty envelope, identical across drivers.
    #[test]
    fn diagnostics_through_wire_is_empty_envelope_for_clean_input() {
        let doc = AozoraDoc::new("plain text".to_owned());
        let json = json::diagnostics(doc.parse().diagnostics());
        assert_eq!(json, r#"{"schemaVersion":1,"data":[]}"#);
    }

    /// Smoke: `decode_auto` round-trips a `Shift_JIS` payload that
    /// `Document::from_bytes` relies on.
    #[test]
    fn decode_auto_round_trips_shift_jis() {
        use aozora::encoding::decode_auto;
        // "青空文庫" in Shift_JIS.
        let sjis = [0x90, 0xC2, 0x8B, 0xF3, 0x95, 0xB6, 0x8C, 0xC9];
        let text = decode_auto(&sjis).expect("valid Shift_JIS");
        assert_eq!(text, "青空文庫");
    }
}
