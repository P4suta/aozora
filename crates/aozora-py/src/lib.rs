//! Python (`PyO3`) driver for the aozora parser.
//!
//! Distributed via [`maturin`](https://www.maturin.rs/) as a wheel:
//!
//! ```bash
//! maturin develop -F extension-module           # build + install in venv
//! maturin build --release -F extension-module   # produce wheel
//! ```
//!
//! ## Move 4 status
//!
//! The `PyO3` binding surface is gated behind the `extension-module`
//! cargo feature so a plain `cargo build --workspace` on the dev
//! environment (which currently lacks Python development headers)
//! still succeeds. Building the actual wheel requires the feature
//! flag and either:
//!
//! - `maturin develop -F extension-module` from inside an active
//!   Python virtualenv, OR
//! - `maturin build --release -F extension-module` against a chosen
//!   Python interpreter.
//!
//! The exposed Python API mirrors the C-ABI surface of `aozora-ffi`
//! (Document handle + `to_html` / `serialize` / `diagnostics`) so a
//! polyglot project can switch between FFI and `PyO3` with no
//! semantic change.

#![forbid(unsafe_code)]

/// Diagnostic projection mirrored from `aozora-ffi` so both drivers
/// (and the WASM driver) emit identical schemas. Public so the
/// `PyO3` bindings can call it; useful in unit tests as well.
#[must_use]
pub fn diagnostics_json_view(diagnostics: &[aozora::Diagnostic]) -> String {
    aozora_wasm::diagnostics_json_view(diagnostics)
}

#[cfg(feature = "extension-module")]
mod bindings {
    use pyo3::exceptions::PyValueError;
    use pyo3::prelude::*;

    /// `PyO3`-facing handle to a parsed Aozora document.
    #[pyclass]
    pub struct Document {
        inner: aozora::Document,
    }

    #[pymethods]
    impl Document {
        /// Construct from a Python `str`.
        #[new]
        fn new(source: &str) -> Self {
            Self {
                inner: aozora::Document::new(source.to_owned()),
            }
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
        fn serialize(&self) -> String {
            self.inner.parse().serialize()
        }

        /// Diagnostics as JSON.
        fn diagnostics(&self) -> PyResult<String> {
            Ok(crate::diagnostics_json_view(self.inner.parse().diagnostics()))
        }

        /// Source byte length.
        fn source_byte_len(&self) -> usize {
            self.inner.source().len()
        }
    }

    /// Module entry point — registered as `aozora` in Python.
    #[pymodule]
    fn aozora(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add_class::<Document>()?;
        m.add_function(wrap_pyfunction!(parse_to_html, m)?)?;
        Ok(())
    }

    /// Convenience: parse + render in one call.
    #[pyfunction]
    fn parse_to_html(source: &str) -> PyResult<String> {
        if source.len() > u32::MAX as usize {
            return Err(PyValueError::new_err(
                "source exceeds 4 GiB (u32::MAX) span limit",
            ));
        }
        Ok(Document::new(source).to_html())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_json_view_works_through_aozora_wasm() {
        let parsed = aozora::parse("abc\u{E001}def");
        let json = diagnostics_json_view(&parsed.diagnostics);
        assert!(json.contains("source_contains_pua"));
    }

    #[test]
    fn diagnostics_json_view_emits_empty_array_for_clean_input() {
        let parsed = aozora::parse("plain text");
        let json = diagnostics_json_view(&parsed.diagnostics);
        assert_eq!(json, "[]");
    }
}
