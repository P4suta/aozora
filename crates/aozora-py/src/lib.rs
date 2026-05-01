//! Python (`PyO3`) driver for the aozora parser.
//!
//! Distributed via [`maturin`](https://www.maturin.rs/) as a wheel.
//! The `PyO3` binding surface is gated behind the `extension-module`
//! cargo feature so a plain `cargo build --workspace` (without
//! Python dev headers installed) still succeeds. Building the actual
//! wheel requires the feature flag plus either:
//!
//! - `maturin develop -F extension-module` from inside a Python venv, or
//! - `maturin build --release -F extension-module` against a chosen
//!   Python interpreter.
//!
//! The exposed Python API mirrors the C-ABI surface of `aozora-ffi`
//! (Document handle + `to_html` / `serialize` / `diagnostics`) so a
//! polyglot project can switch between FFI and `PyO3` with no
//! semantic change. JSON output goes through [`aozora::wire`], the
//! single authority for the cross-driver wire shape.

#![forbid(unsafe_code)]

#[cfg(feature = "extension-module")]
#[allow(
    clippy::too_many_arguments,
    reason = "the #[pyfunction] / #[pymethods] macros expand each fn into a Python ABI wrapper that PyO3 fills with extra context args (Python token, args, kwargs, …). The warning fires on the macro-generated signature, not on user code; per-item allow doesn't reach inside the macro expansion."
)]
mod bindings {
    use aozora::{Document as AozoraDoc, wire};
    use pyo3::exceptions::PyValueError;
    use pyo3::prelude::*;

    /// `PyO3`-facing handle to a parsed Aozora document.
    ///
    /// `unsendable` because [`AozoraDoc`] owns a `bumpalo` arena
    /// with interior `Cell` state — `Send` but not `Sync`. Pinning the
    /// `PyO3` handle to its constructing Python thread reflects the
    /// underlying ownership contract; concurrent access from other
    /// Python threads raises a `RuntimeError` instead of unsound sharing.
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

        /// Diagnostics as JSON. Empty parse →
        /// `{"schema_version":1,"data":[]}`. Wire format defined in
        /// [`aozora::wire`].
        ///
        /// Returns `PyResult<String>` for `PyO3` signature uniformity
        /// even though this method cannot fail — future expansion
        /// (per-diagnostic schema validation) is the natural place to
        /// surface fallible behaviour.
        #[allow(
            clippy::unnecessary_wraps,
            reason = "PyO3 method signatures stay uniform in PyResult so future fallible expansion doesn't break the Python ABI"
        )]
        fn diagnostics(&self) -> PyResult<String> {
            Ok(wire::serialize_diagnostics(
                self.inner.parse().diagnostics(),
            ))
        }

        /// Source-keyed Aozora-node spans as JSON. See
        /// [`aozora::wire::serialize_nodes`] for the schema.
        #[allow(
            clippy::unnecessary_wraps,
            reason = "PyO3 method signatures stay uniform in PyResult so future fallible expansion doesn't break the Python ABI"
        )]
        fn nodes(&self) -> PyResult<String> {
            Ok(wire::serialize_nodes(&self.inner.parse()))
        }

        /// Matched open/close pair links as JSON. See
        /// [`aozora::wire::serialize_pairs`] for the schema.
        #[allow(
            clippy::unnecessary_wraps,
            reason = "PyO3 method signatures stay uniform in PyResult so future fallible expansion doesn't break the Python ABI"
        )]
        fn pairs(&self) -> PyResult<String> {
            Ok(wire::serialize_pairs(&self.inner.parse()))
        }

        /// Source byte length.
        fn source_byte_len(&self) -> usize {
            self.inner.source().len()
        }
    }

    /// Module entry point — registered as `aozora_py` in Python.
    /// Function name must match the cdylib's lib name; see the
    /// `[lib] name` override in Cargo.toml for why we can't call it
    /// `aozora` (hyphenated crate name conflict).
    #[pymodule]
    fn aozora_py(m: &Bound<'_, PyModule>) -> PyResult<()> {
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
    use aozora::{Document as AozoraDoc, wire};

    /// Smoke: PUA collision shows up via `aozora::wire`.
    #[test]
    fn diagnostics_through_wire_emits_pua_kind() {
        let doc = AozoraDoc::new("abc\u{E001}def".to_owned());
        let json = wire::serialize_diagnostics(doc.parse().diagnostics());
        assert!(json.contains("source_contains_pua"), "json: {json}");
    }

    /// Smoke: clean parse → empty envelope, identical across drivers.
    #[test]
    fn diagnostics_through_wire_is_empty_envelope_for_clean_input() {
        let doc = AozoraDoc::new("plain text".to_owned());
        let json = wire::serialize_diagnostics(doc.parse().diagnostics());
        assert_eq!(json, r#"{"schema_version":1,"data":[]}"#);
    }
}
