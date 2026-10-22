//! Provides the compiled `aozora._aozora` Python module behind the `extension-module` feature.
//! JSON accessors use the envelopes defined by [`aozora::json`].

#![forbid(unsafe_code)]

#[cfg(any(feature = "extension-module", test))]
const MAX_SOURCE_BYTES: usize = u32::MAX as usize;

#[cfg(any(feature = "extension-module", test))]
const fn source_len_within_span_limit(byte_len: usize) -> Result<(), &'static str> {
    if byte_len as u64 > MAX_SOURCE_BYTES as u64 {
        return Err("source exceeds 4 GiB (u32::MAX) span limit");
    }
    Ok(())
}

#[cfg(feature = "extension-module")]
#[expect(
    clippy::too_many_arguments,
    reason = "PyO3 emits ABI wrappers as sibling items outside function-level lint attributes"
)]
mod bindings {
    use aozora::{Document as AozoraDoc, decode_auto, decode_sjis as core_decode_sjis, json};
    use pyo3::exceptions::PyValueError;
    use pyo3::prelude::*;

    /// A parsed Aozora document bound to its creating Python thread.
    /// Access from another thread raises `pyo3_runtime.PanicException`.
    #[pyclass(unsendable)]
    #[derive(Debug)]
    pub struct Document {
        inner: AozoraDoc,
    }

    #[pymethods]
    impl Document {
        /// Creates a document from a Python `str`.
        ///
        /// # Errors
        /// Raises `ValueError` if the source's UTF-8 byte length exceeds [`u32::MAX`].
        #[new]
        fn new(source: &str) -> PyResult<Self> {
            crate::source_len_within_span_limit(source.len()).map_err(PyValueError::new_err)?;
            Ok(Self {
                inner: aozora::parse(source)
                    .map_err(|err| PyValueError::new_err(err.to_string()))?,
            })
        }

        /// Creates a document from bytes, preferring UTF-8 and otherwise decoding `Shift_JIS`.
        ///
        /// # Errors
        /// Raises `ValueError` for invalid encoding or decoded source exceeding [`u32::MAX`] bytes.
        #[staticmethod]
        fn from_bytes(data: &[u8]) -> PyResult<Self> {
            let text = decode_auto(data).map_err(|err| PyValueError::new_err(err.to_string()))?;
            crate::source_len_within_span_limit(text.len()).map_err(PyValueError::new_err)?;
            Ok(Self {
                inner: aozora::parse(text.into_owned())
                    .map_err(|err| PyValueError::new_err(err.to_string()))?,
            })
        }

        /// Returns the document's source text.
        #[getter]
        fn source(&self) -> &str {
            self.inner.source()
        }

        /// Renders the document as an HTML string.
        fn to_html(&self) -> String {
            self.inner.snapshot().to_html()
        }

        /// Serializes the document as canonical Aozora source.
        fn to_source(&self) -> String {
            self.inner.snapshot().to_source()
        }

        /// Returns diagnostics as a JSON envelope defined by [`aozora::json::diagnostics`].
        /// A clean parse produces an empty `data` array.
        fn diagnostics_json(&self) -> String {
            json::diagnostics(self.inner.snapshot().diagnostics())
        }

        /// Returns a plain-text diagnostic report, or an empty string for a clean parse.
        fn diagnostics_text(&self) -> String {
            aozora::diagnostics_text(self.inner.source(), self.inner.snapshot().diagnostics())
        }

        /// Returns nodes as a JSON envelope with original-source byte spans.
        /// See [`aozora::json::nodes`] for the wire format.
        fn nodes_json(&self) -> String {
            json::nodes(&self.inner.snapshot())
        }

        /// Returns matched delimiters as a JSON envelope with original-source byte spans.
        /// See [`aozora::json::pairs`] for the wire format.
        fn pairs_json(&self) -> String {
            json::pairs(&self.inner.snapshot())
        }

        /// Returns matched container delimiters as a JSON envelope with original-source byte spans.
        /// See [`aozora::json::container_pairs`] for the wire format.
        fn container_pairs_json(&self) -> String {
            json::container_pairs(&self.inner.snapshot())
        }

        /// Returns gaiji resolutions as a JSON envelope defined by [`aozora::json::gaiji`].
        fn gaiji_json(&self) -> String {
            json::gaiji(&self.inner.snapshot())
        }

        /// Returns the source length in UTF-8 bytes.
        fn source_byte_len(&self) -> usize {
            self.inner.source().len()
        }
    }

    #[pymodule]
    fn _aozora(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add_class::<Document>()?;
        m.add_function(wrap_pyfunction!(parse_to_html, m)?)?;
        m.add_function(wrap_pyfunction!(prewarm, m)?)?;
        m.add_function(wrap_pyfunction!(decode_sjis, m)?)?;
        m.add_function(wrap_pyfunction!(slugs_json, m)?)?;
        m.add_function(wrap_pyfunction!(version, m)?)?;
        m.add_function(wrap_pyfunction!(schema_version, m)?)?;
        Ok(())
    }

    /// Parses source and returns rendered HTML.
    ///
    /// # Errors
    /// Raises `ValueError` if the source's UTF-8 byte length exceeds [`u32::MAX`].
    #[pyfunction]
    fn parse_to_html(source: &str) -> PyResult<String> {
        crate::source_len_within_span_limit(source.len()).map_err(PyValueError::new_err)?;
        Ok(aozora::parse(source)
            .map_err(|err| PyValueError::new_err(err.to_string()))?
            .snapshot()
            .to_html())
    }

    /// Initializes parser tables before the first parse.
    /// Repeated calls have no effect.
    #[pyfunction]
    fn prewarm() {
        aozora::prewarm();
    }

    /// Decodes `Shift_JIS` bytes without replacement characters.
    /// Use [`Document::from_bytes`] to accept UTF-8 as well.
    ///
    /// # Errors
    /// Raises `ValueError` for malformed `Shift_JIS`.
    #[pyfunction]
    fn decode_sjis(data: &[u8]) -> PyResult<String> {
        core_decode_sjis(data).map_err(|err| PyValueError::new_err(err.to_string()))
    }

    /// Returns the document-independent slug catalogue as a JSON envelope.
    /// See [`aozora::json::slugs`] for the wire format.
    #[pyfunction]
    fn slugs_json() -> String {
        json::slugs()
    }

    /// Returns the parser's build version.
    /// Use `aozora.__version__` for the installed Python package version.
    #[pyfunction]
    fn version() -> String {
        env!("AOZORA_VERSION_STRING").to_owned()
    }

    /// Returns the wire schema version carried by each JSON envelope.
    #[pyfunction]
    fn schema_version() -> u32 {
        json::SCHEMA_VERSION
    }
}

#[cfg(test)]
mod tests {
    use aozora::json;

    #[test]
    fn source_len_guard_matches_u32_span_boundary() {
        super::source_len_within_span_limit(0).expect("empty source is in range");
        super::source_len_within_span_limit(u32::MAX as usize)
            .expect("u32::MAX bytes is the inclusive upper bound");
        #[cfg(target_pointer_width = "64")]
        {
            let err = super::source_len_within_span_limit(u32::MAX as usize + 1)
                .expect_err("u32::MAX + 1 bytes must be rejected");
            assert_eq!(err, "source exceeds 4 GiB (u32::MAX) span limit");
        }
    }

    #[test]
    fn diagnostics_through_wire_emits_pua_kind() {
        let doc = aozora::parse("abc\u{E001}def").expect("source fits parser span limit");
        let json = json::diagnostics(doc.snapshot().diagnostics());
        assert!(json.contains("source_contains_pua"), "json: {json}");
    }

    #[test]
    fn diagnostics_through_wire_is_empty_envelope_for_clean_input() {
        let doc = aozora::parse("plain text").expect("source fits parser span limit");
        let json = json::diagnostics(doc.snapshot().diagnostics());
        assert_eq!(
            json,
            format!(r#"{{"schemaVersion":{},"data":[]}}"#, json::SCHEMA_VERSION)
        );
    }

    #[test]
    fn decode_auto_round_trips_shift_jis() {
        use aozora::decode_auto;
        let sjis = [0x90, 0xC2, 0x8B, 0xF3, 0x95, 0xB6, 0x8C, 0xC9];
        let text = decode_auto(&sjis).expect("valid Shift_JIS");
        assert_eq!(text, "青空文庫");
    }
}
